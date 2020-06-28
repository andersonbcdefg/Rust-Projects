mod request;
mod response;

use clap::Clap;
use rand::{Rng, SeedableRng};
use tokio::net::{TcpListener, TcpStream};
use tokio::stream::StreamExt;
use tokio::sync::RwLock;
use std::sync::Arc;
use std::collections::{HashSet, HashMap};
use std::io::{Error, ErrorKind};
use tokio::time::delay_for;
use std::time::Duration;
use std::convert::TryInto;
use http::request::*;

/// Contains information parsed from the command-line invocation of balancebeam. The Clap macros
/// provide a fancy way to automatically construct a command-line argument parser.
#[derive(Clap, Debug)]
#[clap(about = "Fun with load balancing")]
struct CmdOptions {
    #[clap(
        short,
        long,
        about = "IP/port to bind to",
        default_value = "0.0.0.0:1100"
    )]
    bind: String,
    #[clap(short, long, about = "Upstream host to forward requests to")]
    upstream: Vec<String>,
    #[clap(
        long,
        about = "Perform active health checks on this interval (in seconds)",
        default_value = "10"
    )]
    active_health_check_interval: usize,
    #[clap(
    long,
    about = "Path to send request to for active health checks",
    default_value = "/"
    )]
    active_health_check_path: String,
    #[clap(
        long,
        about = "Maximum number of requests to accept per IP per minute (0 = unlimited)",
        default_value = "0"
    )]
    max_requests_per_minute: usize,
}

/// Contains information about the state of balancebeam (e.g. what servers we are currently proxying
/// to, what servers have failed, rate limiting counts, etc.)
///
/// You should add fields to this struct in later milestones.
struct ProxyState {
    /// How frequently we check whether upstream servers are alive (Milestone 4)
    #[allow(dead_code)]
    active_health_check_interval: usize,
    /// Where we should send requests when doing active health checks (Milestone 4)
    #[allow(dead_code)]
    active_health_check_path: String,
    /// Maximum number of requests an individual IP can make in a minute (Milestone 5)
    #[allow(dead_code)]
    max_requests_per_minute: usize,
    /// Addresses of servers that we are proxying to
    upstream_addresses: Vec<String>,
    /// Addresses of dead servers
    dead_upstreams: HashSet<String>,
    /// Number of requests by each client IP
    num_reqs_by_ip: HashMap<String, usize>,

}

#[tokio::main]
async fn main() {
    // Initialize the logging library. You can print log messages using the `log` macros:
    // https://docs.rs/log/0.4.8/log/ You are welcome to continue using print! statements; this
    // just looks a little prettier.
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "debug");
    }
    pretty_env_logger::init();


    // Parse the command line arguments passed to this program
    let options = CmdOptions::parse();
    if options.upstream.len() < 1 {
        log::error!("At least one upstream server must be specified using the --upstream option.");
        std::process::exit(1);
    }

    // Start listening for connections
    let mut listener = TcpListener::bind(&options.bind).await.unwrap();
    log::info!("Listening for requests on {}", options.bind);
    let mut incoming = listener.incoming();


    // Configure state
    let state = Arc::new(RwLock::new(ProxyState {
        upstream_addresses: options.upstream,
        active_health_check_interval: options.active_health_check_interval,
        active_health_check_path: options.active_health_check_path,
        max_requests_per_minute: options.max_requests_per_minute,
        dead_upstreams: HashSet::new(),
        num_reqs_by_ip: HashMap::new(),
    }));

    // Spawn active health checker.
    {
        let state = state.clone();
        tokio::spawn(async move {
                    log::info!("Spawned active health checker.");
                    active_health_checker(state).await;
                });
    }

    // Spawn rate-limiting monitor that empties the counts every minute.
    {
        let state = state.clone();
        tokio::spawn(async move {
                    loop {
                        delay_for(Duration::from_millis(60000)).await;
                        state.write().await.num_reqs_by_ip.clear();
                    }
                });
    }
    

    // Handle incoming connections
    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(stream) => {
                let state = state.clone();
                tokio::spawn(async move {
                    log::debug!("New connection!");
                    handle_connection(stream, state).await;
                });
            }
            Err(_e) => log::error!("Failed to accept a connection.")
        }
        
    }
}

async fn connect_to_upstream(lock: Arc<RwLock<ProxyState>>) -> Result<TcpStream, std::io::Error> {
    let mut rng = rand::rngs::StdRng::from_entropy();
    let mut upstream_idx;
    let mut upstream_ip;
    {
        let state = lock.read().await;
        let valid_upstreams: Vec<String> = state.upstream_addresses.iter().cloned()
            .filter(|x| !state.dead_upstreams.contains(x)).collect();
        if valid_upstreams.len() == 0 {
            return Err(Error::new(ErrorKind::Other, "All upstream servers are unreachable."));
        }
        upstream_idx = rng.gen_range(0, valid_upstreams.len());
        upstream_ip = valid_upstreams[upstream_idx].clone();
    }
    loop {
        match TcpStream::connect(&upstream_ip).await {
            Ok(stream) => return Ok(stream),
            Err(_e) => {
                log::warn!("Upstream server {} is unreachable. Trying another.", upstream_ip);
                let mut state = lock.write().await;
                state.dead_upstreams.insert(upstream_ip.clone());
                let valid_upstreams: Vec<String> = state.upstream_addresses.iter().cloned()
                    .filter(|x| !state.dead_upstreams.contains(x)).collect();
                if valid_upstreams.len() == 0 {
                    return Err(Error::new(ErrorKind::Other, "All upstream servers are unreachable."));
                }
                upstream_idx = rng.gen_range(0, valid_upstreams.len());
                upstream_ip = valid_upstreams[upstream_idx].clone();
            }
        }
    }
}

async fn send_response(client_conn: &mut TcpStream, response: &http::Response<Vec<u8>>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("{} <- {}", client_ip, response::format_response_line(&response));
    if let Err(error) = response::write_to_stream(&response, client_conn).await {
        log::warn!("Failed to send response to client: {}", error);
        return;
    }
}

async fn handle_connection(mut client_conn: TcpStream, lock: Arc<RwLock<ProxyState>>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("Connection received from {}", client_ip);

    // Open a connection to a random destination server
    let mut upstream_conn;
    {
        let lock = lock.clone();
        upstream_conn = match connect_to_upstream(lock).await {
            Ok(stream) => stream,
            Err(_error) => {
                let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                send_response(&mut client_conn, &response).await;
                return;
            }
        };
    }
    
    let upstream_ip = upstream_conn.peer_addr().unwrap().ip().to_string();

    // The client may now send us one or more requests. Keep trying to read requests until the
    // client hangs up or we get an error.
    loop {
        
        // Read the request.
        let mut request = match request::read_from_stream(&mut client_conn).await {
            Ok(request) => request,
            // Handle case where client closed connection and is no longer sending requests
            Err(request::Error::IncompleteRequest(0)) => {
                log::debug!("Client finished sending requests. Shutting down connection");
                return;
            }
            // Handle I/O error in reading from the client
            Err(request::Error::ConnectionError(io_err)) => {
                log::info!("Error reading request from client stream: {}", io_err);
                return;
            }
            Err(error) => {
                log::debug!("Error parsing request: {:?}", error);
                let response = response::make_http_error(match error {
                    request::Error::IncompleteRequest(_)
                    | request::Error::MalformedRequest(_)
                    | request::Error::InvalidContentLength
                    | request::Error::ContentLengthMismatch => http::StatusCode::BAD_REQUEST,
                    request::Error::RequestBodyTooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
                    request::Error::ConnectionError(_) => http::StatusCode::SERVICE_UNAVAILABLE,
                });
                send_response(&mut client_conn, &response).await;
                continue;
            }
        };
        log::info!(
            "{} -> {}: {}",
            client_ip,
            upstream_ip,
            request::format_request_line(&request)
        );

        // If rate limit exceeded, send error to client.
        {   
            let mut state = lock.write().await;
            let max_reqs = state.max_requests_per_minute.clone();
            if let None = state.num_reqs_by_ip.get_mut(&client_ip) {
                state.num_reqs_by_ip.insert(client_ip.clone(), 0);
            } 
            let num = state.num_reqs_by_ip.get_mut(&client_ip).unwrap(); 
            *num += 1;
            log::debug!("reqs: {}; max reqs: {}", num, &max_reqs);
            if *num > max_reqs && max_reqs > 0 {
                let resp = response::make_http_error(http::StatusCode::TOO_MANY_REQUESTS);
                send_response(&mut client_conn, &resp).await;
                continue;
            }
        }

        // Add X-Forwarded-For header so that the upstream server knows the client's IP address.
        // (We're the ones connecting directly to the upstream server, so without this header, the
        // upstream server will only know our IP, not the client's.)
        request::extend_header_value(&mut request, "x-forwarded-for", &client_ip);

        // Forward the request to the server
        if let Err(error) = request::write_to_stream(&request, &mut upstream_conn).await {
            log::error!("Failed to send request to upstream {}: {}", upstream_ip, error);
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
        log::debug!("Forwarded request to server");

        // Read the server's response
        let response = match response::read_from_stream(&mut upstream_conn, request.method()).await {
            Ok(response) => response,
            Err(error) => {
                log::error!("Error reading response from server: {:?}", error);
                let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                send_response(&mut client_conn, &response).await;
                return;
            }
        };
        // Forward the response to the client
        send_response(&mut client_conn, &response).await;
        log::debug!("Forwarded response to client");
    }
}

async fn active_health_checker(lock: Arc<RwLock<ProxyState>>) {
    let interval;
    let path;
    {
        let state = lock.read().await;
        interval = state.active_health_check_interval;
        path = state.active_health_check_path.clone();
    }
    loop {
        delay_for(Duration::from_millis((interval * 1000).try_into().unwrap())).await;
        log::debug!("Time for an active health check!");
        let upstream_addresses;
        {
            let state = lock.read().await;
            upstream_addresses = state.upstream_addresses.clone();
        }
        for u in upstream_addresses {
            let lock = lock.clone();
            let path = path.clone();
            tokio::spawn(async move {
                match TcpStream::connect(&u).await {
                    Ok(mut stream) => {
                        match request::write_to_stream(&build_request(&path, &u), &mut stream).await {
                            Ok(()) => {
                                let resp = response::read_from_stream(&mut stream, &http::Method::GET).await.unwrap();
                                if resp.status().as_u16() == 200 {
                                    log::debug!("Upstream {} ok.", u);
                                    let mut state = lock.write().await;
                                    state.dead_upstreams.remove(&u);
                                } else {
                                    log::warn!("Received non-200 status from {}.", u);
                                    let mut state = lock.write().await;
                                    state.dead_upstreams.insert(u.clone());
                                }
                            }
                            Err(_e) => {
                                log::warn!("Error sending request to {}.", u);
                                let mut state = lock.write().await;
                                state.dead_upstreams.insert(u.clone());
                            }
                        }
                    }
                    Err(_e) => {
                        log::warn!("Upstream server {} is unreachable.", u);
                        let mut state = lock.write().await;
                        state.dead_upstreams.insert(u.clone());
                    }
                }
            });
            
        }
    }
}

fn build_request(path: &String, host: &String) -> Request<Vec<u8>> {
    http::Request::builder()
        .method(http::Method::GET)
        .uri(path)
        .header("Host", host)
        .body(Vec::new())
        .unwrap()
}

