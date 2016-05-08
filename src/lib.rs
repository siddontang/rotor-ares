extern crate rotor;
extern crate c_ares;
#[macro_use]
extern crate log;

use std::thread::{JoinHandle, Builder};
use std::boxed::{Box, FnBox};
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender};

use rotor::{EventSet, PollOpt};
use rotor::{Machine, Response, Scope, EarlyScope};
use rotor::{Config as RotorConfig, Loop, Notifier};
use rotor::void::Void;
use c_ares::{flags, Socket, Channel, Options, SOCKET_BAD, AddressFamily};

pub struct Config {
    pub query_timeout_ms: u64,
    pub max_query_retry: u32;
    pub process_timeout_ms: u64,
    pub max_sockets: u64,
}

impl Config {
    pub fn new() -> Config {
        Config::default()
    }
}

impl Default for Config {
    fn default() -> Config {
        Config{
            query_timeout_ms: 500,
            max_query_retry: 3,
            process_timeout_ms: 500,
            max_sockets: 256,
        }
    }
}

pub type Callback = Box<FnBox<SocketAddr> + Send>;

enum Msg {
    Resolve {
        host: String,
        cb: Callback,
    }
    AresCallback {
        sock: Socket,
        readable: bool,
        writable: bool,
    }
    Quit,
}

enum ResolveFsm {
    Resolver{rx: Receiver<Msg>},
}

struct MsgSender {
    notifer: Notifer,
    tx: Sender<Msg>,
}

impl MsgSender {
    pub fn send(&self, msg: Msg) {
        self.tx.send(msg).unwrap();
        self.notifer.wakeup().unwrap();
    }
}

impl Clone for MsgSender {
    fn clone(&self) -> MsgSender {
        MsgSender{
            notifer: self.notifer.clone(),
            tx: self.tx.clone(),
        }
    }
}

pub struct Resolver {
    cfg: Config,
    handle: Option<JoinHandle<()>>,

    sender: MsgSender,
}

impl Resolver {
    pub fn new(cfg: Config) -> Resolver {
        let mut rotor_cfg = RotorConfig::new();
        rotor_cfg.slab_capacity(cfg.max_sockets);
        rotor_cfg.mio().notify_capacity(cfg.max_sockets);
        let mut rotor_loop = Loop::new(&rotor_cfg).unwrap();
        let mut sender = None;
        {
            let s = &mut sender;
            rotor_loop.add_machine_with(move |scope| {
                let (tx, rx) = mpsc::channel();
                *s = Some(MsgSender{
                    notifer: scope.notifer(),
                    tx: tx,
                });
                Response::ok(ResolveFsm::Resolver{rx:rx})
            }).unwrap();    
        }
       
        let sender = sender.unwrap();
        let mut options = Options::new();
        let sender2 = sender.clone();
        options.set_socket_state_callback(move |sock: Socket, readable: bool, writable: bool| {
            sender2.send(Msg::AresCallback{sock:sock, readable:readable,writable:writable});
        })

        .set_flags(flags::STAYOPEN | flags::EDNS)
        .set_timeout(cfg.query_timeout_ms)
        .set_tries(cfg.max_query_retry);

        let builder = Builder::new().name("resolve thread".to_owned());
        let h = builder.spawn(move || {
            let notifier = scope.notifier();

        }).unwrap();

        Resolver{
            cfg: cfg,
            handle: Some(h),
        }
    }
}

impl Drop for Resolver {
    fn drop(&mut self) {
        self.notifier.send(Msg::Quit);

        let h = self.handle.take().unwrap();

        if let Err(e) = h.join() {
            error!("join resolve thread err {:?}", e);
        }
    }
}

impl Machine for ResolveFsm {
    type Context = Resolver;
    type Seed = ();

    fn create(seed: Self::Seed, scope: &mut Scope<Self::Context>) -> Response<Self, Void> {
        
    }

    fn ready(self, events: EventSet, scope: &mut Scope<Self::Context>) -> Response<Self, Self::Seed> {

    }

    fn spawned(self: scope: &mut Scope<Self::Context>) -> Response<Self, Self::Seed> {

    }

    fn timeout(self, scope: &mut Scope<Self::Context>) -> Response<Self, Self::Seed> {

    }

    fn wakeup(self, scope: &mut Scope<Self::Context>) -> Response<Self, Self::Seed> {
        match self {
            ResolveFsm::Resolver{rx} => {
                if let OK(msg) = rx.try_recv() {
                    match msg {
                        Msg::Quit => {
                            scope.shutdown_loop();
                            Response::done()
                        } 
                        Msg::Resolve{host, cb} => {

                        }
                        Msg::AresCallback{..}=>{
                            
                        }
                    }
                } else {    
                    error!("wakeup resolver, but no message");
                    Response::done()
                }
            }
        }
    }
}

