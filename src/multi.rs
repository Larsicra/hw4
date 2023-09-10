use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use async_channel;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::task::{Waker, Context, Poll, Wake};
use futures::FutureExt;
use futures::future::BoxFuture;
use std::sync::{Condvar, Mutex, Arc};
use std::time::Instant;




scoped_tls::scoped_thread_local!(static SIGNAL: Arc<Signal>);                       // static signal
scoped_tls::scoped_thread_local!(static RUNNABLE: Mutex<VecDeque<Arc<Task>>>);      // static task

//////////////////// block_on

pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut fut = std::pin::pin!(future);
    let signal = Arc::new(Signal::new());
    let waker = Waker::from(signal.clone());
    let mut cx: Context<'_> = Context::from_waker(&waker);

    let runnable = Mutex::new(VecDeque::with_capacity(1024));       // task vec

    let pool = Pool::new(4);

    SIGNAL.set(&signal, || {                            // value and restore signal / task
        RUNNABLE.set(&runnable, || {
            loop {                                          // loop till the future is ready
                if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {       // Poll::Ready, all finished
                    return output;
                }

                while let Some(task) = runnable.lock().unwrap().pop_front() {           // task = runnable.front (previous task)
                    // println!("poped");
                    pool.execute(task);
                }
                signal.wait();
            
                // after poped one, check again
                if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {       // Poll::Ready, all finished
                    // println!("fin");
                    return output;
                }
            }
        })
    })
}



//////////////////////////////

struct Task {
    future: RefCell<BoxFuture<'static, ()>>,
    signal: Arc<Signal>,
}
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl  Wake for Task {                           // when wake
    fn wake(self: Arc<Self>) {                  // push runnable (store the task)
        RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(self.clone()));
        self.signal.notify();
    }
}

fn spawn<F: Future<Output = ()>>(future: F) where F: 'static + Send {              // to spawn, create a new task, and push it into RUNNABLE
    let t = Task{
        future: RefCell::new(future.boxed()),
        signal: Arc::new(Signal::new()),
    };
    RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(Arc::new(t)));
    // println!("pushed");
}

struct Signal {
    state: Mutex<State>,        // lock
    cond: Condvar,              // condition
}
enum State {
    Empty,
    Waiting,
    Notified,
}
impl Signal {
    fn new() -> Self {
        Self {
            state: Mutex::new(State::Empty),
            cond: Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        match *state {
            State::Notified => *state = State::Empty,
            State::Waiting => {
                unreachable!("Multiple threads waiting on the same signal: Open a bug report!");
            }
            State::Empty => {
                *state = State::Waiting;
                while let State::Waiting = *state {
                    state = self.cond.wait(state).unwrap();         // wait to be notified
                }
            }
        }
    }


    fn notify(&self) {
        let mut state = self.state.lock().unwrap();
        // println!("notify");
        match *state {
            State::Notified => {}
            // State::Empty => *state = State::Notified,
            State::Empty => {
                // println!("to notified");
                *state = State::Notified;
            }
            
            State::Waiting => {
                *state = State::Empty;
                self.cond.notify_one();
            }
        }
    }
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

////////////////////////////////////////////////////


struct Worker where {
    _id: usize,                         // id
    t: Option<JoinHandle<()>>,          // thread
}

type Job = Arc<Task>;
enum Message {
    Bye,                                // to end the job
    NewJob(Job),                        // do Job
}


pub struct Pool {
    workers: Vec<Worker>,
    max_num: usize,                     // max number of thread
    sender: mpsc::Sender<Message>       // channel, multi producer single consumer
}                                       // Arc<Mutex::<T>>

impl Pool where {
    pub fn new(max_num: usize) -> Pool {
        if max_num == 0 {
            panic!("max = 0")
        }

        let (tx, rx) = mpsc::channel();                 // channel

        let mut workers = Vec::with_capacity(max_num);
        let receiver = Arc::new(Mutex::new(rx));
        for i in 0..max_num {
            workers.push(Worker::new(i, Arc::clone(&receiver)));
        }
        Pool {workers: workers, max_num: max_num, sender: tx }

    }

    pub fn execute(&self, f: Arc<Task>) {              // create job from f, send job
        let job = Message::NewJob(f);
        self.sender.send(job).unwrap();
    }         // trait for thread::spawn
}

impl Drop for Pool {
    fn drop(&mut self) {
        for _ in 0..self.max_num {
            self.sender.send(Message::Bye).unwrap();                        // end worker
        }
        for w in self.workers.iter_mut() {
            if let Some(t) = w.t.take() {
                t.join().unwrap();                                          // join handle
            }
        }
    }
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let t = thread::spawn(move || {                                         // spawn a new thread and loop for job
            loop {
                // let receiver = receiver.lock().unwrap();    
                // let message = receiver.recv().unwrap();                                // get the message
                let message = receiver.lock().unwrap().recv().unwrap();
                match message {                                                                 // match message
                    Message::NewJob(task) => {   
                        println!("do job from worker {}", id);
                        let waker = Waker::from(task.clone());                                  // get the waker
                        let mut cx = Context::from_waker(&waker);
                        let _ = task.future.borrow_mut().as_mut().poll(&mut cx);                                                                   // do..?
                    },
                    Message::Bye => {       // drop the worker
                        // println!("worker {} finished", id);                                     // end worker
                        break
                    }
                }
                // lock dropped
            }
        });

        Worker {
            _id: id,
            t: Some(t),
        }
    }
}

pub async fn jobs(tx: Arc<async_channel::Sender<()>>) {
    let sta = Instant::now();
    let mut x = 0;
    for _ in 0..1000000000 {
        x *= x;
    }
    let _ = tx.send(()).await;
    let fin = sta.elapsed();
    println!("a job finished with time {:?}", fin);
}

pub async fn test_pool () {
    let (tx, rx) = async_channel::bounded(1);

    let tx_c = Arc::new(tx);

    for _ in 0..6 {
        spawn(jobs(tx_c.clone()));

    }
    for _ in 0..6 {
        let _ = rx.recv().await;
    }
}