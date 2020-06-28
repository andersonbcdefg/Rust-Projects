// Simple Hangman Program
// User gets five incorrect guesses
// Word chosen randomly from words.txt
// Inspiration from: https://doc.rust-lang.org/book/ch02-00-guessing-game-tutorial.html
// This assignment will introduce you to some fundamental syntax in Rust:
// - variable declaration
// - string manipulation
// - conditional statements
// - loops
// - vectors
// - files
// - user input
// We've tried to limit/hide Rust's quirks since we'll discuss those details
// more in depth in the coming lectures.
extern crate rand;
use rand::Rng;
use std::fs;
use std::io;
use std::io::Write;
use std::collections::HashSet;

const NUM_INCORRECT_GUESSES: u32 = 5;
const WORDS_PATH: &str = "words.txt";

fn pick_a_random_word() -> String {
    let file_string = fs::read_to_string(WORDS_PATH).expect("Unable to read file.");
    let words: Vec<&str> = file_string.split('\n').collect();
    String::from(words[rand::thread_rng().gen_range(0, words.len())].trim())
}

fn play_game() {
    let secret_word = pick_a_random_word();
    // Note: given what you know about Rust so far, it's easier to pull characters out of a
    // vector than it is to pull them out of a string. You can get the ith character of
    // secret_word by doing secret_word_chars[i].
    let secret_word_chars: Vec<char> = secret_word.chars().collect();
    // Uncomment for debugging:
    // println!("random word: {}", secret_word);

    let mut guessed_letters: HashSet<char> = HashSet::new();
    let mut guesses = NUM_INCORRECT_GUESSES;

    loop {
        // Create the "partly filled" word.
        let mut template = String::new();
        for ch in &secret_word_chars {
        	if guessed_letters.contains(ch) {
        		template.push(*ch);
        	} else {
        		template.push('_');
        	}
        }

        // Check for win
        if !template.contains('_') {
        	println!("You win! The secret word was '{}'.", secret_word);
        	break;
        }

        // Display the partly-filled word to the user.
        println!("\nYour Word: {}", template);

        // Tell user what letters she has guessed already
        println!("You've already guessed: {:?}", guessed_letters);
        
        // Tell user how many guesses remaining
        println!("You have {} incorrect guesses remaining.", guesses);

        // Solicit a guess from the user.
        let guess: char = loop {
        	print!("Guess a letter!  ");
        	std::io::stdout().flush().expect("Error flushing stdout.");
	        let mut guess = String::new();
	        io::stdin().read_line(&mut guess)
	            .expect("Failed to read line.");
	        let guess: Vec<char> = guess.trim().chars().collect();
	        
	        // Ensure only one character was entered.
	        if guess.len() > 1 {
	        	println!("ERROR: Guess must be one character.");
	        } else {
	        	// Convert the guess to a lowercase char.
	        	let guess: char = guess[0].to_ascii_lowercase();

	        	// Ensure that guess is a letter.
		        if !guess.is_ascii_alphabetic() {
		        	println!("ERROR: Guess must be a letter.");
	        	} else if guessed_letters.contains(&guess) {
	        		println!("ERROR: You've already guessed this letter!");
	        	} else {
	        		break guess;
	        	}
	        }
        };
        
        // Add to guessed letters.
        guessed_letters.insert(guess);

        // Check if the guess is correct.
        if secret_word_chars.contains(&guess) {
        	println!("Correct!");
        } else {
        	println!("Wrong!");
        	guesses = guesses - 1;
        }
        println!("\n-------------------------------------------\n");

        // Check for loss
        if guesses <= 0 {
        	println!("You lost! The secret word was '{}'.", secret_word);
        	break;
        }
    }   
}

fn main() {
	println!("\n-------------------------------------------");
    println!("           WELCOME TO HANGMAN!");
    println!("-------------------------------------------\n");
	loop {
		play_game();
		println!("Play again? (Y/N)");
		let mut play_again = String::with_capacity(5);
		io::stdin().read_line(&mut play_again).expect("Error getting input.");
		let play_again: Vec<char> = play_again.trim().chars().collect();
		let play_again: char = play_again[0].to_ascii_lowercase();
		if play_again == 'n' {
			println!("Thanks for playing!");
			break;
		}

	}
}
