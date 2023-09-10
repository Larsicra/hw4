use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::task::{Waker, Context, Poll, Wake};
use async_channel;
use futures::FutureExt;
use futures::future::BoxFuture;
use std::sync::{Condvar, Mutex, Arc};


scoped_tls::scoped_thread_local!(static SIGNAL: Arc<Signal>);                       // static signal
scoped_tls::scoped_thread_local!(static RUNNABLE: Mutex<VecDeque<Arc<Task>>>);      // static task

//////////////////// block_on
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut fut = std::pin::pin!(future);
    let signal = Arc::new(Signal::new());
    let waker = Waker::from(signal.clone());
    let mut cx: Context<'_> = Context::from_waker(&waker);

    let runnable = Mutex::new(VecDeque::with_capacity(1024));       // task vec

    SIGNAL.set(&signal, || {                            // value and restore signal / task
        RUNNABLE.set(&runnable, || {

            // println!("bef loop");

            loop {                                          // loop till the future is ready
                
                // println!("loop");
                
                if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {       // Poll::Ready, all finished
                    // println!("fin");
                    return output;
                }

                while let Some(task) = runnable.lock().unwrap().pop_front() {           // task = runnable.front (previous task)
                    
                    // println!("pop from RUNNABLE");
                    
                    let waker = Waker::from(task.clone());                                  // get the waker
                    let mut cx = Context::from_waker(&waker);
                    let _ = task.future.borrow_mut().as_mut().poll(&mut cx);                       // poll
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

fn spawn<F: Future<Output = ()> + 'static + Send>(future: F) {              // to spawn, create a new task, and push it into RUNNABLE
    let t = Task{
        future: RefCell::new(future.boxed()),
        signal: Arc::new(Signal::new()),
    };
    RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(Arc::new(t)));
    // println!("pushed into RUNNABLE");
}

//////////////////////////////
pub async fn demo() {
    let (tx, rx) = async_channel::bounded(1);
    spawn(demo2(tx));
    // std::thread::sleep(Duration::from_secs(3));
    println!("demo");
    let _ = rx.recv().await;                // wake after revceive a message
    // println!("recv");
}

async fn demo2(tx: async_channel::Sender<()>) {
    println!("demo2");
    let _ = tx.send(()).await;
    // println!("send");
}

//////////////////////////////////////////////////////////////////
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

////////////////////////////////////////////////////////////
impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

