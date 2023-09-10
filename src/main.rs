use std::time::Instant;

mod multi;
mod single;

fn main() {
    println!("test on single thread Runtime");
    println!("----------------------------------------");
    single::block_on(single::demo());

    
    println!("");
    println!("test on multiple thread Runtime");
    println!("----------------------------------------");
    let start = Instant::now();
    multi::block_on(multi::test_pool());
    let duration = start.elapsed();
    println!("[Result]: ");
    println!("on 4 threads run 6 jobs with time: {:?}", duration);
}
