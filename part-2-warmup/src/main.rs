/* The following exercises were borrowed from Will Crichton's CS 242 Rust lab. */

use std::collections::HashSet;


// I referenced "The Rust Programming Language" chapter on
// vectors when completing these problems. :)
fn main() {
    println!("Hi! Try running \"cargo test\" to run tests.");
}

fn add_n(v: Vec<i32>, n: i32) -> Vec<i32> {
    let mut new_v: Vec<i32> = Vec::new();
    for i in v {
        new_v.push(i + n);
    }
    new_v
}

fn add_n_inplace(v: &mut Vec<i32>, n: i32) {
    for i in v {
        *i += n;
    }
}

fn dedup(v: &mut Vec<i32>) {
    let mut nums: HashSet<i32> = HashSet::new();
    let mut to_remove = Vec::new();
    for (i, &item) in v.iter().enumerate() {
        if !nums.contains(&item) {
            nums.insert(item);
        } else {
            to_remove.push(i);
        }
    }
    for i in (0..to_remove.len()).rev() {
        v.remove(to_remove[i]);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_add_n() {
        assert_eq!(add_n(vec![1], 2), vec![3]);
    }

    #[test]
    fn test_add_n_inplace() {
        let mut v = vec![1];
        add_n_inplace(&mut v, 2);
        assert_eq!(v, vec![3]);
    }

    #[test]
    fn test_dedup() {
        let mut v = vec![3, 1, 0, 1, 4, 4];
        dedup(&mut v);
        assert_eq!(v, vec![3, 1, 0, 4]);
    }
}
