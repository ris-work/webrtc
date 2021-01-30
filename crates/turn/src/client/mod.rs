#[cfg(test)]
mod client_test;

pub mod binding;
pub mod periodic_timer;
pub mod permission;
pub mod relay_conn;
pub mod transaction;

use crate::errors::*;
use crate::proto::{
    chandata::*, data::*, lifetime::*, peeraddr::*, relayaddr::*, reqtrans::*, PROTO_UDP,
};
use binding::*;
use relay_conn::*;
use transaction::*;

use stun::agent::*;
use stun::attributes::*;
use stun::error_code::*;
use stun::fingerprint::*;
use stun::integrity::*;
use stun::message::*;
use stun::textattrs::*;
use stun::xoraddr::*;

use std::sync::Arc;

use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use util::{conn::*, Error};

use async_trait::async_trait;

const DEFAULT_RTO_IN_MS: u16 = 200;
const MAX_DATA_BUFFER_SIZE: usize = u16::MAX as usize; // message size limit for Chromium
const MAX_READ_QUEUE_SIZE: usize = 1024;

//              interval [msec]
// 0: 0 ms      +500
// 1: 500 ms	+1000
// 2: 1500 ms   +2000
// 3: 3500 ms   +4000
// 4: 7500 ms   +8000
// 5: 15500 ms  +16000
// 6: 31500 ms  +32000
// -: 63500 ms  failed

// ClientConfig is a bag of config parameters for Client.
pub struct ClientConfig {
    pub stun_serv_addr: String, // STUN server address (e.g. "stun.abc.com:3478")
    pub turn_serv_addr: String, // TURN server addrees (e.g. "turn.abc.com:3478")
    pub username: String,
    pub password: String,
    pub realm: String,
    pub software: String,
    pub rto_in_ms: u16,
    pub conn: Arc<UdpSocket>, // TODO: change it to Arc<dyn Conn + Send + Sync>
}

struct ClientInternal {
    conn: Arc<UdpSocket>, // TODO: change it to Arc<dyn Conn + Send + Sync>
    stun_serv_addr: String,
    turn_serv_addr: String,
    username: Username,
    password: String,
    realm: Realm,
    integrity: MessageIntegrity,
    software: Software,
    tr_map: Arc<Mutex<TransactionMap>>,
    binding_mgr: Arc<Mutex<BindingManager>>,
    rto_in_ms: u16,
    read_ch_tx: Option<Arc<Mutex<mpsc::Sender<InboundData>>>>,
}

#[async_trait]
impl RelayConnObserver for ClientInternal {
    // turn_server_addr return the TURN server address
    fn turn_server_addr(&self) -> String {
        self.turn_serv_addr.clone()
    }

    // username returns username
    fn username(&self) -> Username {
        self.username.clone()
    }

    // realm return realm
    fn realm(&self) -> Realm {
        self.realm.clone()
    }

    // WriteTo sends data to the specified destination using the base socket.
    async fn write_to(&self, data: &[u8], to: &str) -> Result<usize, Error> {
        let n = self.conn.send_to(data, to).await?;
        Ok(n)
    }

    // PerformTransaction performs STUN transaction
    async fn perform_transaction(
        &mut self,
        msg: &Message,
        to: &str,
        ignore_result: bool,
    ) -> Result<TransactionResult, Error> {
        let tr_key = base64::encode(&msg.transaction_id.0);

        let mut tr = Transaction::new(TransactionConfig {
            key: tr_key.clone(),
            raw: msg.raw.clone(),
            to: to.to_string(),
            interval: self.rto_in_ms,
            ignore_result,
        });
        let result_ch_rx = tr.get_result_channel();

        log::trace!("start {} transaction {} to {}", msg.typ, tr_key, tr.to);
        {
            let mut tm = self.tr_map.lock().await;
            tm.insert(tr_key.clone(), tr);
        }

        self.conn.send_to(&msg.raw, to).await?;

        let conn2 = Arc::clone(&self.conn);
        let tr_map2 = Arc::clone(&self.tr_map);
        {
            let mut tm = self.tr_map.lock().await;
            if let Some(tr) = tm.get(&tr_key) {
                tr.start_rtx_timer(conn2, tr_map2).await;
            }
        }

        // If dontWait is true, get the transaction going and return immediately
        if ignore_result {
            return Ok(TransactionResult::default());
        }

        // wait_for_result waits for the transaction result
        if let Some(mut result_ch_rx) = result_ch_rx {
            match result_ch_rx.recv().await {
                Some(tr) => Ok(tr),
                None => Err(ERR_TRANSACTION_CLOSED.to_owned()),
            }
        } else {
            Err(ERR_WAIT_FOR_RESULT_ON_NON_RESULT_TRANSACTION.to_owned())
        }
    }
}

impl ClientInternal {
    // new returns a new Client instance. listeningAddress is the address and port to listen on, default "0.0.0.0:0"
    async fn new(config: ClientConfig) -> Result<Self, Error> {
        let stun_serv_addr = if config.stun_serv_addr.is_empty() {
            String::new()
        } else {
            log::debug!("resolving {}", config.stun_serv_addr);
            let local_addr = config.conn.local_addr()?;
            let stun_serv = lookup_host(local_addr.is_ipv4(), config.stun_serv_addr).await?;
            log::debug!("local_addr {}, stun_serv: {}", local_addr, stun_serv);
            stun_serv.to_string()
        };

        let turn_serv_addr = if config.turn_serv_addr.is_empty() {
            String::new()
        } else {
            log::debug!("resolving {}", config.turn_serv_addr);
            let local_addr = config.conn.local_addr()?;
            let turn_serv = lookup_host(local_addr.is_ipv4(), config.turn_serv_addr).await?;
            log::debug!("local_addr {}, turn_serv: {}", local_addr, turn_serv);
            turn_serv.to_string()
        };

        Ok(ClientInternal {
            conn: Arc::clone(&config.conn),
            stun_serv_addr,
            turn_serv_addr,
            username: Username::new(ATTR_USERNAME, config.username),
            password: config.password,
            realm: Realm::new(ATTR_REALM, config.realm),
            software: Software::new(ATTR_SOFTWARE, config.software),
            tr_map: Arc::new(Mutex::new(TransactionMap::new())),
            binding_mgr: Arc::new(Mutex::new(BindingManager::new())),
            rto_in_ms: if config.rto_in_ms != 0 {
                config.rto_in_ms
            } else {
                DEFAULT_RTO_IN_MS
            },
            integrity: MessageIntegrity::new_short_term_integrity(String::new()),
            read_ch_tx: None,
        })
    }

    // stun_server_addr return the STUN server address
    fn stun_server_addr(&self) -> String {
        self.stun_serv_addr.clone()
    }

    // Listen will have this client start listening on the relay_conn provided via the config.
    // This is optional. If not used, you will need to call handle_inbound method
    // to supply incoming data, instead.
    async fn listen(&self) -> Result<(), Error> {
        let conn = Arc::clone(&self.conn);
        let stun_serv_str = self.stun_serv_addr.clone();
        let tr_map = Arc::clone(&self.tr_map);
        let read_ch_tx = self.read_ch_tx.clone();
        let binding_mgr = Arc::clone(&self.binding_mgr);

        tokio::spawn(async move {
            let mut buf = vec![0u8; MAX_DATA_BUFFER_SIZE];
            loop {
                //TODO: gracefully exit loop
                let (n, from) = match conn.recv_from(&mut buf).await {
                    Ok((n, from)) => (n, from),
                    Err(err) => {
                        log::debug!("exiting read loop: {}", err);
                        break;
                    }
                };

                if let Err(err) = ClientInternal::handle_inbound(
                    &read_ch_tx,
                    &buf[..n],
                    from,
                    &stun_serv_str,
                    &tr_map,
                    &binding_mgr,
                )
                .await
                {
                    log::debug!("exiting read loop: {}", err);
                    break;
                }
            }
        });

        Ok(())
    }

    // handle_inbound handles data received.
    // This method handles incoming packet demultiplex it by the source address
    // and the types of the message.
    // This return a booleen (handled or not) and if there was an error.
    // Caller should check if the packet was handled by this client or not.
    // If not handled, it is assumed that the packet is application data.
    // If an error is returned, the caller should discard the packet regardless.
    async fn handle_inbound(
        read_ch_tx: &Option<Arc<Mutex<mpsc::Sender<InboundData>>>>,
        data: &[u8],
        from: SocketAddr,
        stun_serv_str: &str,
        tr_map: &Arc<Mutex<TransactionMap>>,
        binding_mgr: &Arc<Mutex<BindingManager>>,
    ) -> Result<(), Error> {
        // +-------------------+-------------------------------+
        // |   Return Values   |                               |
        // +-------------------+       Meaning / Action        |
        // | handled |  error  |                               |
        // |=========+=========+===============================+
        // |  false  |   nil   | Handle the packet as app data |
        // |---------+---------+-------------------------------+
        // |  true   |   nil   |        Nothing to do          |
        // |---------+---------+-------------------------------+
        // |  false  |  error  |     (shouldn't happen)        |
        // |---------+---------+-------------------------------+
        // |  true   |  error  | Error occurred while handling |
        // +---------+---------+-------------------------------+
        // Possible causes of the error:
        //  - Malformed packet (parse error)
        //  - STUN message was a request
        //  - Non-STUN message from the STUN server

        if is_message(data) {
            ClientInternal::handle_stun_message(tr_map, read_ch_tx, data, from).await
        } else if ChannelData::is_channel_data(data) {
            ClientInternal::handle_channel_data(binding_mgr, read_ch_tx, data).await
        } else if !stun_serv_str.is_empty() && from.to_string() == *stun_serv_str {
            // received from STUN server but it is not a STUN message
            Err(ERR_NON_STUNMESSAGE.to_owned())
        } else {
            // assume, this is an application data
            log::trace!("non-STUN/TURN packect, unhandled");
            Ok(())
        }
    }

    async fn handle_stun_message(
        tr_map: &Arc<Mutex<TransactionMap>>,
        read_ch_tx: &Option<Arc<Mutex<mpsc::Sender<InboundData>>>>,
        data: &[u8],
        mut from: SocketAddr,
    ) -> Result<(), Error> {
        let mut msg = Message::new();
        msg.raw = data.to_vec();
        msg.decode()?;

        if msg.typ.class == CLASS_REQUEST {
            return Err(Error::new(format!(
                "{} : {}",
                *ERR_UNEXPECTED_STUNREQUEST_MESSAGE, msg
            )));
        }

        if msg.typ.class == CLASS_INDICATION {
            if msg.typ.method == METHOD_DATA {
                let mut peer_addr = PeerAddress::default();
                peer_addr.get_from(&msg)?;
                from = SocketAddr::new(peer_addr.ip, peer_addr.port);

                let mut data = Data::default();
                data.get_from(&msg)?;

                log::debug!("data indication received from {}", from);

                let _ = ClientInternal::handle_inbound_relay_conn(read_ch_tx, &data.0, from).await;
            }

            return Ok(());
        }

        // This is a STUN response message (transactional)
        // The type is either:
        // - stun.ClassSuccessResponse
        // - stun.ClassErrorResponse

        let tr_key = base64::encode(&msg.transaction_id.0);

        let mut tm = tr_map.lock().await;
        if tm.find(&tr_key).is_none() {
            // silently discard
            log::debug!("no transaction for {}", msg);
            return Ok(());
        }

        if let Some(mut tr) = tm.delete(&tr_key) {
            // End the transaction
            tr.stop_rtx_timer();

            if !tr
                .write_result(TransactionResult {
                    msg,
                    from,
                    retries: tr.retries(),
                    ..Default::default()
                })
                .await
            {
                log::debug!("no listener for msg.raw {:?}", data);
            }
        }

        Ok(())
    }

    async fn handle_channel_data(
        binding_mgr: &Arc<Mutex<BindingManager>>,
        read_ch_tx: &Option<Arc<Mutex<mpsc::Sender<InboundData>>>>,
        data: &[u8],
    ) -> Result<(), Error> {
        let mut ch_data = ChannelData {
            raw: data.to_vec(),
            ..Default::default()
        };
        ch_data.decode()?;

        let addr = ClientInternal::find_addr_by_channel_number(binding_mgr, ch_data.number.0)
            .await
            .ok_or_else(|| ERR_CHANNEL_BIND_NOT_FOUND.to_owned())?;

        log::trace!(
            "channel data received from {} (ch={})",
            addr,
            ch_data.number.0
        );

        let _ = ClientInternal::handle_inbound_relay_conn(read_ch_tx, &ch_data.data, addr).await;

        Ok(())
    }

    // handle_inbound_relay_conn passes inbound data in RelayConn
    async fn handle_inbound_relay_conn(
        read_ch_tx: &Option<Arc<Mutex<mpsc::Sender<InboundData>>>>,
        data: &[u8],
        from: SocketAddr,
    ) -> Result<(), Error> {
        if let Some(read_ch) = &read_ch_tx {
            let tx = read_ch.lock().await;
            if tx
                .try_send(InboundData {
                    data: data.to_vec(),
                    from,
                })
                .is_err()
            {
                log::warn!("receive buffer full");
            }
            Ok(())
        } else {
            Err(ERR_ALREADY_CLOSED.to_owned())
        }
    }

    // Close closes this client
    async fn close(&mut self) {
        self.read_ch_tx.take();
        let mut tm = self.tr_map.lock().await;
        tm.close_and_delete_all();
    }

    // send_binding_request_to sends a new STUN request to the given transport address
    async fn send_binding_request_to(&mut self, to: &str) -> Result<SocketAddr, Error> {
        let msg = {
            let attrs: Vec<Box<dyn Setter>> = if !self.software.text.is_empty() {
                vec![
                    Box::new(TransactionId::new()),
                    Box::new(BINDING_REQUEST),
                    Box::new(self.software.clone()),
                ]
            } else {
                vec![Box::new(TransactionId::new()), Box::new(BINDING_REQUEST)]
            };

            let mut msg = Message::new();
            msg.build(&attrs)?;
            msg
        };

        let tr_res = self.perform_transaction(&msg, to, false).await?;

        let mut refl_addr = XORMappedAddress::default();
        refl_addr.get_from(&tr_res.msg)?;

        Ok(SocketAddr::new(refl_addr.ip, refl_addr.port))
    }

    // send_binding_request sends a new STUN request to the STUN server
    async fn send_binding_request(&mut self) -> Result<SocketAddr, Error> {
        if self.stun_serv_addr.is_empty() {
            Err(ERR_STUNSERVER_ADDRESS_NOT_SET.to_owned())
        } else {
            self.send_binding_request_to(&self.stun_serv_addr.clone())
                .await
        }
    }

    // find_addr_by_channel_number returns a peer address associated with the
    // channel number on this UDPConn
    async fn find_addr_by_channel_number(
        binding_mgr: &Arc<Mutex<BindingManager>>,
        ch_num: u16,
    ) -> Option<SocketAddr> {
        let bm = binding_mgr.lock().await;
        if let Some(b) = bm.find_by_number(ch_num) {
            Some(b.addr)
        } else {
            None
        }
    }

    // Allocate sends a TURN allocation request to the given transport address
    async fn allocate(&mut self) -> Result<RelayConnConfig, Error> {
        if self.read_ch_tx.is_some() {
            return Err(ERR_ONE_ALLOCATE_ONLY.to_owned());
        }

        let mut msg = Message::new();
        msg.build(&[
            Box::new(TransactionId::new()),
            Box::new(MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST)),
            Box::new(RequestedTransport {
                protocol: PROTO_UDP,
            }),
            Box::new(FINGERPRINT),
        ])?;

        let tr_res = self
            .perform_transaction(&msg, &self.turn_serv_addr.clone(), false)
            .await?;
        let res = tr_res.msg;

        // Anonymous allocate failed, trying to authenticate.
        let nonce = Nonce::get_from_as(&res, ATTR_NONCE)?;
        self.realm = Realm::get_from_as(&res, ATTR_REALM)?;

        self.integrity = MessageIntegrity::new_long_term_integrity(
            self.username.text.clone(),
            self.realm.text.clone(),
            self.password.clone(),
        );

        // Trying to authorize.
        msg.build(&[
            Box::new(TransactionId::new()),
            Box::new(MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST)),
            Box::new(RequestedTransport {
                protocol: PROTO_UDP,
            }),
            Box::new(self.username.clone()),
            Box::new(self.realm.clone()),
            Box::new(nonce.clone()),
            Box::new(self.integrity.clone()),
            Box::new(FINGERPRINT),
        ])?;

        let tr_res = self
            .perform_transaction(&msg, &self.turn_serv_addr.clone(), false)
            .await?;
        let res = tr_res.msg;

        if res.typ.class == CLASS_ERROR_RESPONSE {
            let mut code = ErrorCodeAttribute::default();
            let result = code.get_from(&res);
            if result.is_err() {
                return Err(Error::new(format!("{}", res.typ)));
            } else {
                return Err(Error::new(format!("{} (error {})", res.typ, code)));
            }
        }

        // Getting relayed addresses from response.
        let mut relayed = RelayedAddress::default();
        relayed.get_from(&res)?;
        let relayed_addr = SocketAddr::new(relayed.ip, relayed.port);

        // Getting lifetime from response
        let mut lifetime = Lifetime::default();
        lifetime.get_from(&res)?;

        let (read_ch_tx, read_ch_rx) = mpsc::channel(MAX_READ_QUEUE_SIZE);
        self.read_ch_tx = Some(Arc::new(Mutex::new(read_ch_tx)));

        Ok(RelayConnConfig {
            relayed_addr,
            integrity: self.integrity.clone(),
            nonce,
            lifetime: lifetime.0,
            binding_mgr: Arc::clone(&self.binding_mgr),
            read_ch_rx: Arc::new(Mutex::new(read_ch_rx)),
        })
    }
}

// Client is a STUN server client
#[derive(Clone)]
pub struct Client {
    client_internal: Arc<Mutex<ClientInternal>>,
}

impl Client {
    pub async fn new(config: ClientConfig) -> Result<Self, Error> {
        let ci = ClientInternal::new(config).await?;
        Ok(Client {
            client_internal: Arc::new(Mutex::new(ci)),
        })
    }

    pub async fn listen(&self) -> Result<(), Error> {
        let ci = self.client_internal.lock().await;
        ci.listen().await
    }

    pub async fn allocate(&self) -> Result<impl Conn, Error> {
        let config = {
            let mut ci = self.client_internal.lock().await;
            ci.allocate().await?
        };

        Ok(RelayConn::new(Arc::clone(&self.client_internal), config))
    }

    pub async fn close(&self) -> Result<(), Error> {
        let mut ci = self.client_internal.lock().await;
        ci.close().await;
        Ok(())
    }

    // send_binding_request_to sends a new STUN request to the given transport address
    pub async fn send_binding_request_to(&self, to: &str) -> Result<SocketAddr, Error> {
        let mut ci = self.client_internal.lock().await;
        ci.send_binding_request_to(to).await
    }

    // send_binding_request sends a new STUN request to the STUN server
    pub async fn send_binding_request(&self) -> Result<SocketAddr, Error> {
        let mut ci = self.client_internal.lock().await;
        ci.send_binding_request().await
    }
}
