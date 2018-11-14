extern crate bundler;
extern crate minion;

use bundler::Runtime;
use minion::Cancellable;

fn main() {
    let mut r = Runtime::new().unwrap();
    r.run().unwrap()
}
