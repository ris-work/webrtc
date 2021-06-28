use webrtc_ice as ice;

use ice::agent::agent_config::AgentConfig;
use ice::agent::Agent;
use ice::candidate::*;
use ice::error::Error;
use ice::network_type::*;
use ice::state::*;

use anyhow::Result;
use clap::{App, AppSettings, Arg};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Method, Request, Response, Server, StatusCode};
use rand::{thread_rng, Rng};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use util::Conn;

#[macro_use]
extern crate lazy_static;

type SenderType = Arc<Mutex<mpsc::Sender<String>>>;
type ReceiverType = Arc<Mutex<mpsc::Receiver<String>>>;

lazy_static! {
    // ErrUnknownType indicates an error with Unknown info.
    static ref REMOTE_AUTH_CHANNEL: (SenderType, ReceiverType ) = {
        let (tx, rx) = mpsc::channel::<String>(3);
        (Arc::new(Mutex::new(tx)), Arc::new(Mutex::new(rx)))
    };

    static ref REMOTE_CAND_CHANNEL: (SenderType, ReceiverType) = {
        let (tx, rx) = mpsc::channel::<String>(10);
        (Arc::new(Mutex::new(tx)), Arc::new(Mutex::new(rx)))
    };
}

// HTTP Listener to get ICE Credentials/Candidate from remote Peer
async fn remote_handler(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    //println!("received {:?}", req);
    match (req.method(), req.uri().path()) {
        (&Method::POST, "/remoteAuth") => {
            let full_body =
                match std::str::from_utf8(&hyper::body::to_bytes(req.into_body()).await?) {
                    Ok(s) => s.to_owned(),
                    Err(err) => panic!("{}", err),
                };
            let tx = REMOTE_AUTH_CHANNEL.0.lock().await;
            //println!("body: {:?}", full_body);
            let _ = tx.send(full_body).await;

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }

        (&Method::POST, "/remoteCandidate") => {
            let full_body =
                match std::str::from_utf8(&hyper::body::to_bytes(req.into_body()).await?) {
                    Ok(s) => s.to_owned(),
                    Err(err) => panic!("{}", err),
                };
            let tx = REMOTE_CAND_CHANNEL.0.lock().await;
            //println!("body: {:?}", full_body);
            let _ = tx.send(full_body).await;

            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            Ok(response)
        }

        // Return the 404 Not Found for other routes.
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

// Controlled Agent:
//      cargo run --color=always --package webrtc-ice --example ping_pong
// Controlling Agent:
//      cargo run --color=always --package webrtc-ice --example ping_pong -- --controlling

#[tokio::main]
async fn main() -> Result<()> {
    /*env_logger::Builder::new()
    .format(|buf, record| {
        writeln!(
            buf,
            "{}:{} [{}] {} - {}",
            record.file().unwrap_or("unknown"),
            record.line().unwrap_or(0),
            record.level(),
            chrono::Local::now().format("%H:%M:%S.%6f"),
            record.args()
        )
    })
    .filter(None, log::LevelFilter::Trace)
    .init();*/

    let mut app = App::new("ICE Demo")
        .version("0.1.0")
        .author("Rain Liu <yliu@webrtc.rs>")
        .about("An example of ICE")
        .setting(AppSettings::DeriveDisplayOrder)
        .setting(AppSettings::SubcommandsNegateReqs)
        .arg(
            Arg::with_name("FULLHELP")
                .help("Prints more detailed help information")
                .long("fullhelp"),
        )
        .arg(
            Arg::with_name("controlling")
                .takes_value(false)
                .long("controlling")
                .help("is ICE Agent controlling"),
        );

    let matches = app.clone().get_matches();

    if matches.is_present("FULLHELP") {
        app.print_long_help().unwrap();
        std::process::exit(0);
    }

    let is_controlling = matches.is_present("controlling");

    let (local_http_port, remote_http_port) = if is_controlling {
        (9000, 9001)
    } else {
        (9001, 9000)
    };

    println!("Listening on http://localhost:{}", local_http_port);
    tokio::spawn(async move {
        let addr = ([0, 0, 0, 0], local_http_port).into();
        let service =
            make_service_fn(|_| async { Ok::<_, hyper::Error>(service_fn(remote_handler)) });
        let server = Server::bind(&addr).serve(service);
        // Run this server for... forever!
        if let Err(e) = server.await {
            eprintln!("server error: {}", e);
        }
    });

    if is_controlling {
        println!("Local Agent is controlling");
    } else {
        println!("Local Agent is controlled");
    };
    println!("Press 'Enter' when both processes have started");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input)?;

    let ice_agent = Arc::new(
        Agent::new(AgentConfig {
            network_types: vec![NetworkType::Udp4],
            ..Default::default()
        })
        .await?,
    );

    let client = Arc::new(Client::new());

    // When we have gathered a new ICE Candidate send it to the remote peer
    let client2 = Arc::clone(&client);
    ice_agent
        .on_candidate(Box::new(
            move |c: Option<Arc<dyn Candidate + Send + Sync>>| {
                let client3 = Arc::clone(&client2);
                Box::pin(async move {
                    if let Some(c) = c {
                        println!("{}", c.marshal());

                        let req = match Request::builder()
                            .method(Method::POST)
                            .uri(format!(
                                "http://localhost:{}/remoteCandidate",
                                remote_http_port
                            ))
                            .header("content-type", "application/json")
                            .body(Body::from(c.marshal()))
                        {
                            Ok(req) => req,
                            Err(err) => {
                                println!("{}", err);
                                return;
                            }
                        };
                        let resp = match client3.request(req).await {
                            Ok(resp) => resp,
                            Err(err) => {
                                println!("{}", err);
                                return;
                            }
                        };
                        println!("Response from remoteCandidate: {}", resp.status());
                    }
                })
            },
        ))
        .await;

    // When ICE Connection state has change print to stdout
    ice_agent
        .on_connection_state_change(Box::new(|c: ConnectionState| {
            Box::pin(async move {
                println!("ICE Connection State has changed: {}", c);
            })
        }))
        .await;

    // Get the local auth details and send to remote peer
    let (local_ufrag, local_pwd) = ice_agent.get_local_user_credentials().await;

    let req = match Request::builder()
        .method(Method::POST)
        .uri(format!("http://localhost:{}/remoteAuth", remote_http_port))
        .header("content-type", "application/json")
        .body(Body::from(format!("{}:{}", local_ufrag, local_pwd)))
    {
        Ok(req) => req,
        Err(err) => return Err(Error::new(format!("{}", err)).into()),
    };
    let resp = match client.request(req).await {
        Ok(resp) => resp,
        Err(err) => return Err(Error::new(format!("{}", err)).into()),
    };
    println!("Response from remoteAuth: {}", resp.status());

    let (remote_ufrag, remote_pwd) = {
        let mut rx = REMOTE_AUTH_CHANNEL.1.lock().await;
        if let Some(s) = rx.recv().await {
            let fields: Vec<String> = s.split(':').map(|s| s.to_string()).collect();
            (fields[0].clone(), fields[1].clone())
        } else {
            panic!("rx.recv() empty");
        }
    };
    println!("remote_ufrag: {}, remote_pwd: {}", remote_ufrag, remote_pwd);

    let ice_agent2 = Arc::clone(&ice_agent);
    tokio::spawn(async move {
        let mut rx = REMOTE_CAND_CHANNEL.1.lock().await;
        while let Some(s) = rx.recv().await {
            if let Ok(c) = ice_agent2.unmarshal_remote_candidate(s).await {
                println!("add_remote_candidate: {}", c);
                let c: Arc<dyn Candidate + Send + Sync> = Arc::new(c);
                let _ = ice_agent2.add_remote_candidate(&c).await;
            }
        }
    });

    ice_agent.gather_candidates().await?;
    println!("Connecting...");

    let (_cancel_tx, cancel_rx) = mpsc::channel(1);
    // Start the ICE Agent. One side must be controlled, and the other must be controlling
    let conn: Arc<dyn Conn + Send + Sync> = if is_controlling {
        ice_agent.dial(cancel_rx, remote_ufrag, remote_pwd).await?
    } else {
        ice_agent
            .accept(cancel_rx, remote_ufrag, remote_pwd)
            .await?
    };

    // Send messages in a loop to the remote peer
    let conn_tx = Arc::clone(&conn);
    tokio::spawn(async move {
        const RANDOM_STRING: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;

            let val: String = (0..15)
                .map(|_| {
                    let idx = thread_rng().gen_range(0..RANDOM_STRING.len());
                    RANDOM_STRING[idx] as char
                })
                .collect();

            let _ = conn_tx.send(val.as_bytes()).await;

            println!("Sent: '{}'", val);
        }
    });

    // Receive messages in a loop from the remote peer
    let mut buf = vec![0u8; 1500];
    while let Ok(n) = conn.recv(&mut buf).await {
        println!("Received: '{}'", std::str::from_utf8(&buf[..n]).unwrap());
    }

    Ok(())
}
