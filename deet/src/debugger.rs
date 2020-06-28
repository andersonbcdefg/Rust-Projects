use nix::sys::wait::WaitPidFlag;
use nix::sys::ptrace;
use nix::sys::signal;
use std::fs::File;
use std::io::Read;
use crate::dwarf_data::{DwarfData, Error as DwarfError};
use crate::debugger_command::DebuggerCommand;
use crate::inferior::Inferior;
use crate::inferior::Status;
use rustyline::error::ReadlineError;
use rustyline::Editor;
#[allow(unused_imports)]
use std::convert::TryInto;

pub struct Debugger {
    target: String,
    history_path: String,
    readline: Editor<()>,
    inferior: Option<Inferior>,
    debug_data: DwarfData,
    breakpoints: Vec<usize>
}

impl Debugger {
    /// Initializes the debugger.
    pub fn new(target: &str) -> Debugger {
        let debug_data = match DwarfData::from_file(target) {
            Ok(val) => val,
            Err(DwarfError::ErrorOpeningFile) => {
                println!("Could not open file: {}", target);
                std::process::exit(1);
            },
            Err(DwarfError::DwarfFormatError(err)) => {
                println!("Could not read debugging symbols from {}: {:?}", target, err);
                std::process::exit(1);
            }
        };

        debug_data.print();

        let history_path = format!("{}/.deet_history", std::env::var("HOME").unwrap());
        let mut readline = Editor::<()>::new();
        // Attempt to load history from ~/.deet_history if it exists
        let _ = readline.load_history(&history_path);

        Debugger {
            target: target.to_string(),
            history_path,
            readline,
            inferior: None,
            debug_data,
            breakpoints: Vec::new()
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.get_next_command() {
                DebuggerCommand::Run(args) => {
                    // If there's a stopped process, kill it before running a new one.
                    if let Some(_i) = &self.inferior {
                        self.inferior.as_mut().unwrap().kill().expect("Error terminating previous program.");
                    }

                    // Run the program we want to debug as a child process.
                    println!("Starting program: {}", self.target);
                    if let Some(inferior) = Inferior::new(&self.target, &args, &self.breakpoints) {
                        // Save the inferior process
                        self.inferior = Some(inferior);
                        self.resume();
                    } else {
                        println!("Error starting subprocess.");
                    }
                },
                DebuggerCommand::Quit => {
                    // If there's a stopped process running in the debugger, kill it quitting.
                    if let Some(_i) = &self.inferior {
                        let pid = self.inferior.as_ref().unwrap().pid();
                        println!("A debugging session is active.");
                        println!("Process {} will be killed.", pid);
                        let quit = self.get_yes_no("Quit anyway? (y or n) ");
                        if quit {
                            self.inferior.as_mut().unwrap().kill().expect("Error terminating child.");
                            return;
                        }
                    } else {
                        return;
                    }
                },
                DebuggerCommand::Continue => {
                    if let None = self.inferior {
                        println!("No stopped program to resume.");
                    } else {
                        self.resume();
                    }
                },
                DebuggerCommand::Backtrace => { 
                    if let None = self.inferior {
                        println!("No program is running.");
                    } else {
                        self.backtrace();
                    }
                },
                DebuggerCommand::Break(s) => {
                    self.set_breakpoint(&s)
                }

            }
        }
    }

    /// Helper function to resume stopped child
    pub fn resume(&mut self) {
        {
            let inf = self.inferior.as_mut().unwrap();
            let pid = inf.pid();
            let regs = ptrace::getregs(pid).unwrap();
            let ip: usize = regs.rip.try_into().unwrap();

            // If stopped at a breakpoint, take one step and restore it.
            if self.breakpoints.contains(&ip) {
                ptrace::step(pid, None).unwrap();
                if let Ok(status) = inf.wait(Some(WaitPidFlag::empty())) {
                    match status {
                        Status::Stopped(sig, _ip) => {
                            if sig == signal::SIGTRAP {
                                inf.write_byte(ip, 0xcc).unwrap();
                            } else {
                                panic!("Stopped after ptrace::step due to signal other than SIGTRAP.");
                            }
                        },
                        _ => {
                            self.handle_stop(status);
                        }
                    }
                }

            }
        }
        // Resume the child, wait for and handle stop.
        let inf = self.inferior.as_mut().unwrap();
        let pid = inf.pid();
        ptrace::cont(pid, None).unwrap();
        if let Ok(status) = inf.wait(Some(WaitPidFlag::empty())) {
            self.handle_stop(status);
        } else {
            println!("Error resuming process.");
        }
    }

    /// Helper function to handle stopped child
    pub fn handle_stop(&mut self, status: Status) {
        match status {
            Status::Stopped(sig, ip) => {
                // Check if we stopped at a breakpoint.
                let break_addr = ip - 1;
                let pid = self.inferior.as_ref().unwrap().pid();
                
                // If so, restore the byte and rewind %rip.
                if self.breakpoints.contains(&break_addr) {
                    println!("Program stopped at breakpoint {:#010x} ({})", 
                        break_addr, sig);
                    let inf = self.inferior.as_mut().unwrap();
                    inf.write_byte(break_addr, inf.bp_map.get(&break_addr).unwrap().clone()).unwrap();
                    let mut regs = ptrace::getregs(pid).unwrap();
                    regs.rip = break_addr.try_into().unwrap();
                    ptrace::setregs(pid, regs).unwrap();
                } else {
                    println!("\nProgram stopped (signal {})", sig);
                }
                let (function, line) = self.get_func_and_line(ip);
                println!("Stopped in function {} in {}", function, line);
                
                let line_num = match self.debug_data.get_line_from_addr(ip) {
                    Some(l) => l.number,
                    None => {
                        return;
                    }
                };
                let filename = format!("{}.c", self.target);
                let mut file = match File::open(filename) {
                    Ok(file) => file,
                    Err(_) => panic!("File not found!"),
                };
                let mut contents = String::new();
                file.read_to_string(&mut contents).unwrap();
                let lines: Vec<&str> = contents.split("\n").collect();
                println!("{}     {}", line_num, lines[line_num - 1]);
            }
            Status::Exited(stat) => {
                println!("\nExited with status {}", stat);
                self.inferior = None;
            },
            Status::Signaled(sig) => {
                println!("\nTerminated due to signal {}", sig);
                self.inferior = None;
            },
        }
    }

    /// Helper function to perform a backtrace
    pub fn backtrace(&self) {
        // Read the registers of the tracee
        let pid = self.inferior.as_ref().unwrap().pid();
        let regs = ptrace::getregs(pid).unwrap();
        
        // Print the backtrace
        let mut ip: usize = regs.rip.try_into().unwrap();
        let mut bp: usize = regs.rbp.try_into().unwrap();
        let mut stack_idx = 0;
        loop {
            // Get function and line information
            let (function, line) = self.get_func_and_line(ip);
            println!("#{}  {:#010x} in {} at {}", stack_idx, ip, function, line);
            if function == "main" {break;}

            // Get next ip and bp, increment stackframe number
            ip = match ptrace::read(pid, (bp + 8) as ptrace::AddressType) {
                Ok(val) => val.try_into().unwrap(),
                Err(_e) => {
                    println!("[...]\nERROR: Unable to complete backtrace.");
                    break;
                }
            };
            bp = match ptrace::read(pid, bp as ptrace::AddressType) {
                Ok(val) => val.try_into().unwrap(),
                Err(_e) => {
                    println!("[...]\nERROR: Unable to complete backtrace.");
                    break;
                }
            };
            stack_idx += 1;
        }
        
    }

    /// Helper to get function and line number from DWARF data
    pub fn get_func_and_line(&self, ip: usize) -> (String, String) {
        // Get function name
        let function = match self.debug_data.get_function_from_addr(ip) {
            Some(val) => val,
            None => String::from("[untraceable function]")
        };
        // Get file and line number
        let line = match self.debug_data.get_line_from_addr(ip) {
            Some(val) => format!("{}", val),
            None => String::from("[unknown location]")
        };
        (function, line)
    }

    pub fn set_breakpoint(&mut self, inp: &str) {
        // Parse the input string
        let breakpoint: usize;
        if inp.starts_with("*") {
            breakpoint = match Debugger::parse_address(&inp[1..]) {
                Some(addr) => addr,
                None => {
                    println!("ERROR: {} is not a well-formed address.", &inp[1..]);
                    return;
                },
            };
        } else {
            breakpoint = match inp.parse::<usize>().ok() {
                Some(line) => {
                    match self.debug_data.get_addr_for_line(None, line) {
                        Some(addr) => addr,
                        None => {
                            println!("ERROR: {} is not a valid line number.", inp);
                            return;
                        }
                    }
                },
                None => {
                    match self.debug_data.get_addr_for_function(None, inp) {
                        Some(addr) => addr,
                        None => {
                            println!("ERROR: {} is not a valid function name.", inp);
                            return;
                        }
                    }
                }
            }
        }
        
        // Check if breakpoint has already been set.
        if self.breakpoints.contains(&breakpoint) {
            println!("Breakpoint already set here ({})", breakpoint);
            return;
        }
        
        // If not, set the breakpoint
        println!("Setting breakpoint {} at {:#010x}", 
                self.breakpoints.len(), breakpoint);
            
        // If child hasnt started yet, then we just add it to the array 
        // of breakpoints, the child will write it when it starts.
        // If child has started, we have to call the child's method to
        // write the breakpoint into memory, as well as add to array.
        if let Some(inf) = &mut self.inferior {
            let bpt = vec![breakpoint];
            if let Err(_) = inf.write_breakpoints(&bpt) {
                return;
            }
        }
        self.breakpoints.push(breakpoint);
        
    }

    pub fn parse_address(addr: &str) -> Option<usize> {
        let addr_without_0x = if addr.to_lowercase().starts_with("0x") {
            &addr[2..]
        } else {
            &addr
        };
        usize::from_str_radix(addr_without_0x, 16).ok()
    }

    fn get_yes_no(&mut self, prompt: &str) -> bool {
        let mut p = prompt;
        loop {
            match self.readline.readline(p) {
                Err(ReadlineError::Interrupted) => {
                    p = "Please type (y/n) ";
                    continue;
                }
                Err(ReadlineError::Eof) => {
                    p = "Please type (y/n) ";
                    continue;
                }
                Err(err) => {
                    panic!("Unexpected I/O error: {:?}", err);
                }
                Ok(line) => {
                    p = "Please type (y/n) ";
                    if line.trim().len() == 0 {
                        continue;
                    }
                    self.readline.add_history_entry(line.as_str());
                    if let Err(err) = self.readline.save_history(&self.history_path) {
                        println!(
                            "Warning: failed to save history file at {}: {}",
                            self.history_path, err
                        );
                    }
                    let tokens: Vec<&str> = line.split_whitespace().collect();
                    match tokens[0] {
                        "y" | "yes" | "Y" | "Yes" => return true,
                        "n" | "no" | "N" | "No" => return false,
                        _ => println!("Unrecognized command."),
                    }
                }
            }
        }
    }

    /// This function prompts the user to enter a command, and continues re-prompting until the user
    /// enters a valid command. It uses DebuggerCommand::from_tokens to do the command parsing.
    ///
    /// You don't need to read, understand, or modify this function.
    fn get_next_command(&mut self) -> DebuggerCommand {
        loop {
            // Print prompt and get next line of user input
            match self.readline.readline("(deet) ") {
                Err(ReadlineError::Interrupted) => {
                    // User pressed ctrl+c. We're going to ignore it
                    println!("Type \"quit\" to exit");
                }
                Err(ReadlineError::Eof) => {
                    // User pressed ctrl+d, which is the equivalent of "quit" for our purposes
                    return DebuggerCommand::Quit;
                }
                Err(err) => {
                    panic!("Unexpected I/O error: {:?}", err);
                }
                Ok(line) => {
                    if line.trim().len() == 0 {
                        continue;
                    }
                    self.readline.add_history_entry(line.as_str());
                    if let Err(err) = self.readline.save_history(&self.history_path) {
                        println!(
                            "Warning: failed to save history file at {}: {}",
                            self.history_path, err
                        );
                    }
                    let tokens: Vec<&str> = line.split_whitespace().collect();
                    if let Some(cmd) = DebuggerCommand::from_tokens(&tokens) {
                        return cmd;
                    } else {
                        println!("Unrecognized command.");
                    }
                }
            }
        }
    }
}
