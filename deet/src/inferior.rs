use nix::sys::ptrace;
use nix::sys::signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::process::Child;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::mem::size_of;
use std::collections::HashMap;


pub enum Status {
    /// Indicates inferior stopped. Contains the signal that stopped the process, as well as the
    /// current instruction pointer that it is stopped at.
    Stopped(signal::Signal, usize),

    /// Indicates inferior exited normally. Contains the exit status code.
    Exited(i32),

    /// Indicates the inferior exited due to a signal. Contains the signal that killed the
    /// process.
    Signaled(signal::Signal),
}

/// This function calls ptrace with PTRACE_TRACEME to enable debugging on a process. You should use
/// pre_exec with Command to call this in the child process.
fn child_traceme() -> Result<(), std::io::Error> {
    ptrace::traceme().or(Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "ptrace TRACEME failed",
    )))
}

fn align_addr_to_word(addr: usize) -> usize {
    addr & (-(size_of::<usize>() as isize) as usize)
}

pub struct Inferior {
    child: Child,
    pub bp_map: HashMap<usize, u8>
}

impl Inferior {
    /// Attempts to start a new inferior process. Returns Some(Inferior) if successful,
    /// or None if an error is encountered.
    pub fn new(target: &str, args: &Vec<String>, breakpoints: &Vec<usize>) -> Option<Inferior> {
        let mut cmd = Command::new(target);
        unsafe {
            cmd.pre_exec(child_traceme);
        }
        if let Ok(child) = cmd.args(args).spawn() {
            
            // Create inferior struct with child and empty HashMap.
            let mut inf = Inferior{child, bp_map: HashMap::new()};
            
            // Wait for child to terminate due to SIGTRAP, if it does,
            // write the breakpoints and return it. If not, return None.
            if let Ok(status) = inf.wait(Some(WaitPidFlag::empty())) {
                if let Status::Stopped(sig, _ip) = status {
                    if sig != signal::SIGTRAP {
                        None
                    } else {
                        if let Err(_) = inf.write_breakpoints(breakpoints) {
                            println!("Remove invalid breakpoint by quitting deet and reopening.");
                        }
                        Some(inf)
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Returns the pid of this inferior.
    pub fn pid(&self) -> Pid {
        nix::unistd::Pid::from_raw(self.child.id() as i32)
    }

    /// Kills and reaps the child process
    pub fn kill(&mut self) -> Result<Status, nix::Error> {
        self.child.kill().unwrap();
        self.wait(Some(WaitPidFlag::empty()))
    }

    /// Calls waitpid on this inferior and returns a Status to indicate the state of the process
    /// after the waitpid call.
    pub fn wait(&self, options: Option<WaitPidFlag>) -> Result<Status, nix::Error> {
        Ok(match waitpid(self.pid(), options)? {
            WaitStatus::Exited(_pid, exit_code) => Status::Exited(exit_code),
            WaitStatus::Signaled(_pid, signal, _core_dumped) => Status::Signaled(signal),
            WaitStatus::Stopped(_pid, signal) => {
                let regs = ptrace::getregs(self.pid())?;
                Status::Stopped(signal, regs.rip as usize)
            }
            other => panic!("waitpid returned unexpected status: {:?}", other),
        })
    }

    // Writes breakpoint(s) to memory and saves the original addresses in
    // the hashmap bp_map.
    pub fn write_breakpoints(&mut self, breakpoints: &Vec<usize>) -> Result<(), nix::Error> {
        for bpt in breakpoints {
            if self.bp_map.contains_key(bpt) {
                println!("Breakpoint already set at this location.");
            } else {
                match self.write_byte(*bpt, 0xcc) {
                    Ok(orig_byte) => {
                        self.bp_map.insert(bpt.clone(), orig_byte);
                    },
                    Err(e) => {
                        println!("Unable to set breakpoint at {:#014x}: {:?}", bpt, e);
                        return Err(e);
                    }
                }
                
            }
        }
        Ok(())
    }

    // Write a provided byte to a provided address
    pub fn write_byte(&mut self, addr: usize, val: u8) -> Result<u8, nix::Error> {
        let aligned_addr = align_addr_to_word(addr);
        let byte_offset = addr - aligned_addr;
        let word = ptrace::read(self.pid(), aligned_addr as ptrace::AddressType)? as u64;
        let orig_byte = (word >> 8 * byte_offset) & 0xff;
        let masked_word = word & !(0xff << 8 * byte_offset);
        let updated_word = masked_word | ((val as u64) << 8 * byte_offset);
        ptrace::write(
            self.pid(),
            aligned_addr as ptrace::AddressType,
            updated_word as *mut std::ffi::c_void,
        )?;
        Ok(orig_byte as u8)
    }
}
