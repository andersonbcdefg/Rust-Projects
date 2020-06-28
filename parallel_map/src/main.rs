use crossbeam_channel::bounded;
use std::{thread, time};

fn parallel_map<T, U, F>(input_vec: Vec<T>, num_threads: usize, f: F) -> Vec<U>
where
    F: FnOnce(T) -> U + Send + Copy + 'static,
    T: Send + 'static,
    U: Send + 'static + Default,
{
    let mut output_vec: Vec<U> = Vec::with_capacity(input_vec.len());
    for _ in 0..input_vec.len() {
        output_vec.push(Default::default());
    }
    // TODO: implement parallel map!

    // This channel will be used to send inputs to the threads to operate on.
    // Will expect messages of the form (index, input).
    let (send_to_thread, receive_from_parent) = bounded(input_vec.len());
    
    // This channel will be used to send outputs from calling the function back
    // to the parent. Will expect messages of the form (index, output).
    let (send_to_parent, receive_from_thread) = bounded(input_vec.len());

    // Spawn all the threads
    let mut threads = Vec::new();
    for i in 0..num_threads {
        let send_to_parent = send_to_parent.clone();
        let receive_from_parent = receive_from_parent.clone();
        threads.push(thread::spawn(move || {
            while let Ok(input_pair) = receive_from_parent.recv() {
                let (index, input) = input_pair;
                let output = f(input);
                let output_pair = (index, output);
                send_to_parent.send(output_pair).expect("Parent receiver unexpectedly closed!");
            }
            drop(send_to_parent);
        }))
    }
    drop(send_to_parent);

    // Send numbers to the threads, then drop that sender
    for (index, item) in input_vec.into_iter().enumerate() {
        let input_pair = (index, item);
        send_to_thread.send(input_pair).expect("Thread receiver unexpectedly closed!");
    }
    drop(send_to_thread);

    // Receive results from threads, save to output vector
    while let Ok(output_pair) = receive_from_thread.recv() {
        let (index, output) = output_pair;
        output_vec[index] = output;
    }
    output_vec
}

fn main() {
    let v = vec![6, 7, 8, 9, 10, 1, 2, 3, 4, 5, 12, 18, 11, 5, 20];
    let squares = parallel_map(v, 10, |num| {
        println!("{} squared is {}", num, num * num);
        thread::sleep(time::Duration::from_millis(500));
        num * num
    });
    println!("squares: {:?}", squares);
    let v = vec![6, 7, 8, 9, 10, 1, 2, 3, 4, 5, 12, 18, 11, 5, 20];
    let mut vs = Vec::new();
    for n in v.iter() {
        vs.push(n * n);
    }
    print!("compare: {:?}\n", vs);
}
