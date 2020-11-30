#[cfg(test)]
mod conn_test;

use crate::alert::*;
use crate::application_data::*;
use crate::cipher_suite::*;
use crate::config::*;
use crate::content::*;
use crate::curve::named_curve::NamedCurve;
use crate::errors::*;
use crate::extension::extension_use_srtp::*;
use crate::flight::flight0::*;
use crate::flight::flight1::*;
use crate::flight::flight5::*;
use crate::flight::flight6::*;
use crate::flight::*;
use crate::fragment_buffer::*;
use crate::handshake::handshake_cache::*;
use crate::handshake::handshake_header::HandshakeHeader;
use crate::handshake::*;
use crate::handshaker::*;
use crate::record_layer::record_layer_header::*;
use crate::record_layer::*;
use crate::signature_hash_algorithm::parse_signature_schemes;
use crate::state::*;

use transport::replay_detector::*;

use std::collections::HashMap;
use std::io::{BufReader, BufWriter};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::Arc;

use log::*;

use tokio::net::*;
use tokio::sync::{mpsc, Mutex};
use tokio::time;
use tokio::time::{timeout, Duration};

use util::Error;

pub(crate) const INITIAL_TICKER_INTERVAL: time::Duration = time::Duration::from_secs(1);
pub(crate) const COOKIE_LENGTH: usize = 20;
pub(crate) const DEFAULT_NAMED_CURVE: NamedCurve = NamedCurve::P256; //TODO: NamedCurve::X25519;
pub(crate) const INBOUND_BUFFER_SIZE: usize = 8192;
// Default replay protection window is specified by RFC 6347 Section 4.1.2.6
pub(crate) const DEFAULT_REPLAY_PROTECTION_WINDOW: usize = 64;

lazy_static! {
    pub static ref INVALID_KEYING_LABELS: HashMap<&'static str, bool> = {
        let mut map = HashMap::new();
        map.insert("client finished", true);
        map.insert("server finished", true);
        map.insert("master secret", true);
        map.insert("key expansion", true);
        map
    };
}

struct ConnReaderContext {
    is_client: bool,
    replay_protection_window: usize,
    replay_detector: Vec<Box<dyn ReplayDetector + Send>>,
    decrypted_tx: mpsc::Sender<Result<Vec<u8>, Error>>,
    encrypted_packets: Vec<Vec<u8>>,
    fragment_buffer: FragmentBuffer,
    cache: HandshakeCache,
    cipher_suite: Arc<Mutex<Option<Box<dyn CipherSuite + Send + Sync>>>>,
    remote_epoch: Arc<AtomicU16>,
    handshake_tx: mpsc::Sender<mpsc::Sender<()>>,
    handshake_done_rx: mpsc::Receiver<()>,
}

// Conn represents a DTLS connection
pub(crate) struct Conn {
    pub(crate) cache: HandshakeCache, // caching of handshake messages for verifyData generation
    decrypted_rx: mpsc::Receiver<Result<Vec<u8>, Error>>, // Decrypted Application Data or error, pull by calling `Read`
    pub(crate) state: State,                              // Internal state

    handshake_completed_successfully: Arc<AtomicBool>,
    connection_closed_by_user: bool,
    // closeLock              sync.Mutex
    closed: bool, //  *closer.Closer
    //handshakeLoopsFinished sync.WaitGroup

    //readDeadline  :deadline.Deadline,
    //writeDeadline :deadline.Deadline,

    //log logging.LeveledLogger
    /*
    reading               chan struct{}
    handshakeRecv         chan chan struct{}
    cancelHandshaker      func()
    cancelHandshakeReader func()
    */
    pub(crate) current_flight: Box<dyn Flight + Send + Sync>,
    pub(crate) flights: Option<Vec<Packet>>,
    pub(crate) cfg: HandshakeConfig,
    pub(crate) retransmit: bool,
    pub(crate) handshake_rx: mpsc::Receiver<mpsc::Sender<()>>,

    pub(crate) packet_tx: Arc<mpsc::Sender<Vec<Packet>>>,
    pub(crate) handle_queue_tx: mpsc::Sender<mpsc::Sender<()>>,
    pub(crate) handshake_done_tx: Option<mpsc::Sender<()>>,
}

impl Conn {
    pub async fn new(
        udp_socket: UdpSocket,
        mut config: Config,
        is_client: bool,
        initial_state: Option<State>,
    ) -> Result<Self, Error> {
        validate_config(&config)?;

        let local_cipher_suites: Vec<CipherSuiteID> = parse_cipher_suites(
            &config.cipher_suites,
            config.psk.is_none(),
            config.psk.is_some(),
        )?
        .iter()
        .map(|cs| cs.id())
        .collect();

        let sigs: Vec<u16> = config.signature_schemes.iter().map(|x| *x as u16).collect();
        let local_signature_schemes = parse_signature_schemes(&sigs, config.insecure_hashes)?;

        let retransmit_interval = if config.flight_interval != Duration::from_secs(0) {
            config.flight_interval
        } else {
            INITIAL_TICKER_INTERVAL
        };

        /*
           loggerFactory := config.LoggerFactory
           if loggerFactory == nil {
               loggerFactory = logging.NewDefaultLoggerFactory()
           }

           logger := loggerFactory.NewLogger("dtls")
        */
        let maximum_transmission_unit = if config.mtu == 0 {
            DEFAULT_MTU
        } else {
            config.mtu
        };

        let replay_protection_window = if config.replay_protection_window == 0 {
            DEFAULT_REPLAY_PROTECTION_WINDOW
        } else {
            config.replay_protection_window
        };

        let server_name = config.server_name.clone();
        // Use host from conn address when server_name is not provided
        // TODO:
        /*if is_client && server_name == "" && next_conn.RemoteAddr() != nil {
            remoteAddr := nextConn.RemoteAddr().String()
            var host string
            host, _, err = net.SplitHostPort(remoteAddr)
            if err != nil {
                server_name = remoteAddr
            } else {
                server_name = host
            }
        }*/

        let cfg = HandshakeConfig {
            local_psk_callback: config.psk.take(),
            local_psk_identity_hint: config.psk_identity_hint.clone(),
            local_cipher_suites,
            local_signature_schemes,
            extended_master_secret: config.extended_master_secret,
            local_srtp_protection_profiles: config.srtp_protection_profiles.clone(),
            server_name,
            client_auth: config.client_auth,
            local_certificates: config.certificates.clone(),
            insecure_skip_verify: config.insecure_skip_verify,
            verify_peer_certificate: config.verify_peer_certificate.take(),
            //rootCAs: config.RootCAs,
            //clientCAs: config.ClientCAs,
            retransmit_interval,
            //log: logger,
            initial_epoch: 0,
            ..Default::default()
        };

        let (state, flight, initial_fsm_state) = if let Some(state) = initial_state {
            let flight = if is_client {
                Box::new(Flight5 {}) as Box<dyn Flight + Send + Sync>
            } else {
                Box::new(Flight6 {}) as Box<dyn Flight + Send + Sync>
            };

            (state, flight, HandshakeState::Finished)
        } else {
            let flight = if is_client {
                Box::new(Flight1 {}) as Box<dyn Flight + Send + Sync>
            } else {
                Box::new(Flight0 {}) as Box<dyn Flight + Send + Sync>
            };

            (
                State {
                    is_client,
                    ..Default::default()
                },
                flight,
                HandshakeState::Preparing,
            )
        };

        let (decrypted_tx, decrypted_rx) = mpsc::channel(1);
        let (handshake_tx, handshake_rx) = mpsc::channel(1);
        let (handshake_done_tx, handshake_done_rx) = mpsc::channel(1);
        let (packet_tx, packet_rx) = mpsc::channel(1);
        let (handle_queue_tx, mut handle_queue_rx) = mpsc::channel(1);

        let packet_tx1 = Arc::new(packet_tx);
        let packet_tx2 = Arc::clone(&packet_tx1);
        let next_conn_rx = Arc::new(udp_socket);
        let next_conn_tx = Arc::clone(&next_conn_rx);
        let cache = HandshakeCache::new();
        let cache1 = cache.clone();
        let cache2 = cache.clone();
        let handshake_completed_successfully = Arc::new(AtomicBool::new(false));
        let handshake_completed_successfully2 = Arc::clone(&handshake_completed_successfully);

        let mut c = Conn {
            cache,
            decrypted_rx,
            state,
            handshake_completed_successfully,
            connection_closed_by_user: false,
            closed: false,

            current_flight: flight,
            flights: None,
            cfg,
            retransmit: false,
            handshake_rx,
            packet_tx: packet_tx1,
            handle_queue_tx,
            handshake_done_tx: Some(handshake_done_tx),
        };

        let cipher_suite1 = Arc::clone(&c.state.cipher_suite);
        let sequence_number = Arc::clone(&c.state.local_sequence_number);

        tokio::spawn(async move {
            let _ = Conn::handle_outgoing_packets(
                next_conn_tx,
                packet_rx,
                cache1,
                is_client,
                sequence_number,
                cipher_suite1,
                maximum_transmission_unit,
            )
            .await;
        });

        let local_epoch = Arc::clone(&c.state.local_epoch);
        let remote_epoch = Arc::clone(&c.state.remote_epoch);
        let cipher_suite2 = Arc::clone(&c.state.cipher_suite);

        tokio::spawn(async move {
            let mut buf = vec![0u8; INBOUND_BUFFER_SIZE];
            let mut ctx = ConnReaderContext {
                is_client,
                replay_protection_window,
                replay_detector: vec![],
                decrypted_tx,
                encrypted_packets: vec![],
                fragment_buffer: FragmentBuffer::new(),
                cache: cache2,
                cipher_suite: cipher_suite2,
                remote_epoch,
                handshake_tx,
                handshake_done_rx,
            };

            //trace!("before enter read_and_buffer: {}] ", srv_cli_str(is_client));
            loop {
                let _ = Conn::read_and_buffer(
                    &mut ctx,
                    &next_conn_rx,
                    &packet_tx2,
                    &mut handle_queue_rx,
                    &mut buf,
                    &local_epoch,
                    &handshake_completed_successfully2,
                )
                .await;
            }
        });

        // Do handshake
        c.handshake(initial_fsm_state).await?;

        trace!("Handshake Completed");

        Ok(c)
    }

    // Read reads data from the connection.
    pub async fn read(&mut self, p: &mut [u8], duration: Option<Duration>) -> Result<usize, Error> {
        if !self.is_handshake_completed_successfully() {
            return Err(ERR_HANDSHAKE_IN_PROGRESS.clone());
        }

        loop {
            let rx = if let Some(d) = duration {
                match timeout(d, self.decrypted_rx.recv()).await {
                    Ok(rx) => rx,
                    Err(_) => return Err(ERR_DEADLINE_EXCEEDED.clone()),
                }
            } else {
                self.decrypted_rx.recv().await
            };

            if let Some(out) = rx {
                match out {
                    Ok(val) => {
                        if p.len() < val.len() {
                            return Err(ERR_BUFFER_TOO_SMALL.clone());
                        }
                        p[..val.len()].copy_from_slice(&val);
                        return Ok(val.len());
                    }
                    Err(err) => return Err(err),
                };
            } else {
                continue;
            }
        }
    }

    // Write writes len(p) bytes from p to the DTLS connection
    pub async fn write(&mut self, p: &[u8], duration: Option<Duration>) -> Result<usize, Error> {
        if self.is_connection_closed() {
            return Err(ERR_CONN_CLOSED.clone());
        }

        if !self.is_handshake_completed_successfully() {
            return Err(ERR_HANDSHAKE_IN_PROGRESS.clone());
        }

        let pkts = vec![Packet {
            record: RecordLayer::new(
                PROTOCOL_VERSION1_2,
                self.get_local_epoch(),
                Content::ApplicationData(ApplicationData { data: p.to_vec() }),
            ),
            should_encrypt: true,
            reset_local_sequence_number: false,
        }];

        if let Some(d) = duration {
            if timeout(d, self.write_packets(pkts)).await.is_err() {
                return Err(ERR_DEADLINE_EXCEEDED.clone());
            }
        } else {
            self.write_packets(pkts).await?;
        }

        Ok(p.len())
    }

    // Close closes the connection.
    pub fn close(&self) -> Result<(), Error> {
        //err := c.close(true)
        //c.handshakeLoopsFinished.Wait()
        //return err
        Ok(())
    }

    // ConnectionState returns basic DTLS details about the connection.
    // Note that this replaced the `Export` function of v1.
    pub async fn connection_state(&self) -> State {
        self.state.clone().await
    }

    // selected_srtpprotection_profile returns the selected SRTPProtectionProfile
    pub fn selected_srtpprotection_profile(&self) -> SRTPProtectionProfile {
        //c.lock.RLock()
        //defer c.lock.RUnlock()

        self.state.srtp_protection_profile
    }

    pub(crate) async fn notify(
        &mut self,
        level: AlertLevel,
        desc: AlertDescription,
    ) -> Result<(), Error> {
        self.write_packets(vec![Packet {
            record: RecordLayer::new(
                PROTOCOL_VERSION1_2,
                self.get_local_epoch(),
                Content::Alert(Alert {
                    alert_level: level,
                    alert_description: desc,
                }),
            ),
            should_encrypt: self.is_handshake_completed_successfully(),
            reset_local_sequence_number: false,
        }])
        .await
    }

    pub(crate) async fn write_packets(&mut self, pkts: Vec<Packet>) -> Result<(), Error> {
        self.packet_tx.send(pkts).await?;

        Ok(())
    }

    async fn handle_outgoing_packets(
        next_conn: Arc<UdpSocket>,
        mut packet_rx: mpsc::Receiver<Vec<Packet>>,
        mut cache: HandshakeCache,
        is_client: bool,
        sequence_number: Arc<AtomicU64>,
        cipher_suite: Arc<Mutex<Option<Box<dyn CipherSuite + Send + Sync>>>>,
        maximum_transmission_unit: usize,
    ) -> Result<(), Error> {
        let mut local_sequence_number = vec![];

        loop {
            let rx = packet_rx.recv().await;
            if let Some(mut pkts) = rx {
                let mut raw_packets = vec![];
                for p in &mut pkts {
                    if let Content::Handshake(h) = &p.record.content {
                        let mut handshake_raw = vec![];
                        {
                            let mut writer = BufWriter::<&mut Vec<u8>>::new(handshake_raw.as_mut());
                            p.record.marshal(&mut writer)?;
                        }
                        trace!(
                            "Send [handshake:{}] -> {} (epoch: {}, seq: {})",
                            srv_cli_str(is_client),
                            h.handshake_header.handshake_type.to_string(),
                            p.record.record_layer_header.epoch,
                            h.handshake_header.message_sequence
                        );
                        cache
                            .push(
                                handshake_raw[RECORD_LAYER_HEADER_SIZE..].to_vec(),
                                p.record.record_layer_header.epoch,
                                h.handshake_header.message_sequence,
                                h.handshake_header.handshake_type,
                                is_client,
                            )
                            .await;

                        let raw_handshake_packets = Conn::process_handshake_packet(
                            &mut local_sequence_number,
                            &sequence_number,
                            &cipher_suite,
                            maximum_transmission_unit,
                            p,
                            h,
                        )
                        .await?;
                        raw_packets.extend_from_slice(&raw_handshake_packets);
                    } else {
                        let raw_packet = Conn::process_packet(
                            &mut local_sequence_number,
                            &sequence_number,
                            &cipher_suite,
                            p,
                        )
                        .await?;
                        raw_packets.push(raw_packet);
                    }
                }
                if raw_packets.is_empty() {
                    continue;
                }

                let compacted_raw_packets =
                    compact_raw_packets(&raw_packets, maximum_transmission_unit);

                for compacted_raw_packets in &compacted_raw_packets {
                    next_conn.send(compacted_raw_packets).await?;
                }
            }
        }
    }

    async fn process_packet(
        local_sequence_number: &mut Vec<u64>,
        sequence_number: &Arc<AtomicU64>,
        cipher_suite: &Arc<Mutex<Option<Box<dyn CipherSuite + Send + Sync>>>>,
        p: &mut Packet,
    ) -> Result<Vec<u8>, Error> {
        let epoch = p.record.record_layer_header.epoch as usize;
        let seq = {
            while local_sequence_number.len() <= epoch {
                local_sequence_number.push(0);
            }
            local_sequence_number[epoch] += 1;
            sequence_number.store(local_sequence_number[epoch], Ordering::Relaxed);
            local_sequence_number[epoch] - 1
        };
        if seq > MAX_SEQUENCE_NUMBER {
            // RFC 6347 Section 4.1.0
            // The implementation must either abandon an association or rehandshake
            // prior to allowing the sequence number to wrap.
            return Err(ERR_SEQUENCE_NUMBER_OVERFLOW.clone());
        }
        p.record.record_layer_header.sequence_number = seq;

        let mut raw_packet = vec![];
        {
            let mut writer = BufWriter::<&mut Vec<u8>>::new(raw_packet.as_mut());
            p.record.marshal(&mut writer)?;
        }

        if p.should_encrypt {
            let cipher_suite = cipher_suite.lock().await;
            if let Some(cipher_suite) = &*cipher_suite {
                raw_packet = cipher_suite.encrypt(&p.record.record_layer_header, &raw_packet)?;
            }
        }

        Ok(raw_packet)
    }

    async fn process_handshake_packet(
        local_sequence_number: &mut Vec<u64>,
        sequence_number: &Arc<AtomicU64>,
        cipher_suite: &Arc<Mutex<Option<Box<dyn CipherSuite + Send + Sync>>>>,
        maximum_transmission_unit: usize,
        p: &Packet,
        h: &Handshake,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let mut raw_packets = vec![];

        let handshake_fragments = Conn::fragment_handshake(maximum_transmission_unit, h)?;

        let epoch = p.record.record_layer_header.epoch as usize;

        while local_sequence_number.len() <= epoch {
            local_sequence_number.push(0);
        }

        for handshake_fragment in &handshake_fragments {
            let seq = {
                local_sequence_number[epoch] += 1;
                sequence_number.store(local_sequence_number[epoch], Ordering::Relaxed);
                local_sequence_number[epoch] - 1
            };
            if seq > MAX_SEQUENCE_NUMBER {
                return Err(ERR_SEQUENCE_NUMBER_OVERFLOW.clone());
            }

            let record_layer_header = RecordLayerHeader {
                protocol_version: p.record.record_layer_header.protocol_version,
                content_type: p.record.record_layer_header.content_type,
                content_len: handshake_fragment.len() as u16,
                epoch: p.record.record_layer_header.epoch,
                sequence_number: seq,
            };

            let mut record_layer_header_bytes = vec![];
            {
                let mut writer = BufWriter::<&mut Vec<u8>>::new(record_layer_header_bytes.as_mut());
                record_layer_header.marshal(&mut writer)?;
            }

            //p.record.record_layer_header = record_layer_header;

            let mut raw_packet = vec![];
            raw_packet.extend_from_slice(&record_layer_header_bytes);
            raw_packet.extend_from_slice(&handshake_fragment);
            if p.should_encrypt {
                let cipher_suite = cipher_suite.lock().await;
                if let Some(cipher_suite) = &*cipher_suite {
                    raw_packet = cipher_suite.encrypt(&record_layer_header, &raw_packet)?;
                }
            }

            raw_packets.push(raw_packet);
        }

        Ok(raw_packets)
    }

    fn fragment_handshake(
        maximum_transmission_unit: usize,
        h: &Handshake,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let mut content = vec![];
        {
            let mut writer = BufWriter::<&mut Vec<u8>>::new(content.as_mut());
            h.handshake_message.marshal(&mut writer)?;
        }

        let mut fragmented_handshakes = vec![];

        let mut content_fragments = split_bytes(&content, maximum_transmission_unit);
        if content_fragments.is_empty() {
            content_fragments = vec![vec![]];
        }

        let mut offset = 0;
        for content_fragment in &content_fragments {
            let content_fragment_len = content_fragment.len();

            let handshake_header_fragment = HandshakeHeader {
                handshake_type: h.handshake_header.handshake_type,
                length: h.handshake_header.length,
                message_sequence: h.handshake_header.message_sequence,
                fragment_offset: offset as u32,
                fragment_length: content_fragment_len as u32,
            };

            offset += content_fragment_len;

            let mut handshake_header_fragment_raw = vec![];
            {
                let mut writer =
                    BufWriter::<&mut Vec<u8>>::new(handshake_header_fragment_raw.as_mut());
                handshake_header_fragment.marshal(&mut writer)?;
            }

            let mut fragmented_handshake = vec![];
            fragmented_handshake.extend_from_slice(&handshake_header_fragment_raw);
            fragmented_handshake.extend_from_slice(&content_fragment);

            fragmented_handshakes.push(fragmented_handshake);
        }

        Ok(fragmented_handshakes)
    }

    pub(crate) fn set_handshake_completed_successfully(&mut self) {
        self.handshake_completed_successfully
            .store(true, Ordering::Relaxed);
    }

    pub(crate) fn is_handshake_completed_successfully(&self) -> bool {
        self.handshake_completed_successfully
            .load(Ordering::Relaxed)
    }

    async fn read_and_buffer(
        ctx: &mut ConnReaderContext,
        next_conn: &Arc<UdpSocket>,
        packet_tx: &Arc<mpsc::Sender<Vec<Packet>>>,
        handle_queue_rx: &mut mpsc::Receiver<mpsc::Sender<()>>,
        buf: &mut [u8],
        local_epoch: &Arc<AtomicU16>,
        handshake_completed_successfully: &Arc<AtomicBool>,
    ) -> Result<(), Error> {
        let mut has_handshake = false;
        let n = next_conn.recv(buf).await?;
        let pkts = unpack_datagram(&buf[..n])?;

        for pkt in pkts {
            let (hs, alert, mut err) = Conn::handle_incoming_packet(ctx, pkt, true).await;
            if let Some(alert) = alert {
                let alert_err = packet_tx
                    .send(vec![Packet {
                        record: RecordLayer::new(
                            PROTOCOL_VERSION1_2,
                            local_epoch.load(Ordering::Relaxed),
                            Content::Alert(Alert {
                                alert_level: alert.alert_level,
                                alert_description: alert.alert_description,
                            }),
                        ),
                        should_encrypt: handshake_completed_successfully.load(Ordering::Relaxed),
                        reset_local_sequence_number: false,
                    }])
                    .await;

                if let Err(alert_err) = alert_err {
                    if err.is_none() {
                        err = Some(Error::new(alert_err.to_string()));
                    }
                }

                if alert.alert_level == AlertLevel::Fatal
                    || alert.alert_description == AlertDescription::CloseNotify
                {
                    return Err(Error::new("Alert is Fatal or Close Notify".to_owned()));
                }
            }

            if let Some(err) = err {
                return Err(err);
            }

            if hs {
                has_handshake = true
            }
        }

        if has_handshake {
            let (done_tx, mut done_rx) = mpsc::channel(1);

            tokio::select! {
                _ = ctx.handshake_tx.send(done_tx) => {
                    let mut wait_done_rx = true;
                    while wait_done_rx{
                        tokio::select!{
                            _ = done_rx.recv() => {
                                // If the other party may retransmit the flight,
                                // we should respond even if it not a new message.
                                wait_done_rx = false;
                            }
                            done = handle_queue_rx.recv() => {
                                trace!("recv handle_queue: {} ", srv_cli_str(ctx.is_client));

                                let pkts = ctx.encrypted_packets.drain(..).collect();
                                Conn::handle_queued_packets(ctx, packet_tx, local_epoch, handshake_completed_successfully, pkts).await?;

                                drop(done);
                            }
                        }
                    }
                }
                _ = ctx.handshake_done_rx.recv() => {}
            }
        }

        Ok(())
    }

    async fn handle_queued_packets(
        ctx: &mut ConnReaderContext,
        packet_tx: &Arc<mpsc::Sender<Vec<Packet>>>,
        local_epoch: &Arc<AtomicU16>,
        handshake_completed_successfully: &Arc<AtomicBool>,
        pkts: Vec<Vec<u8>>,
    ) -> Result<(), Error> {
        for p in pkts {
            let (_, alert, mut err) = Conn::handle_incoming_packet(ctx, p, false).await; // don't re-enqueue
            if let Some(alert) = alert {
                let alert_err = packet_tx
                    .send(vec![Packet {
                        record: RecordLayer::new(
                            PROTOCOL_VERSION1_2,
                            local_epoch.load(Ordering::Relaxed),
                            Content::Alert(Alert {
                                alert_level: alert.alert_level,
                                alert_description: alert.alert_description,
                            }),
                        ),
                        should_encrypt: handshake_completed_successfully.load(Ordering::Relaxed),
                        reset_local_sequence_number: false,
                    }])
                    .await;

                if let Err(alert_err) = alert_err {
                    if err.is_none() {
                        err = Some(Error::new(alert_err.to_string()));
                    }
                }
                if alert.alert_level == AlertLevel::Fatal
                    || alert.alert_description == AlertDescription::CloseNotify
                {
                    return Err(Error::new("Alert is Fatal or Close Notify".to_owned()));
                }
            }

            if let Some(err) = err {
                return Err(err);
            }
        }

        Ok(())
    }

    async fn handle_incoming_packet(
        ctx: &mut ConnReaderContext,
        mut pkt: Vec<u8>,
        enqueue: bool,
    ) -> (bool, Option<Alert>, Option<Error>) {
        let mut reader = BufReader::new(pkt.as_slice());
        let h = match RecordLayerHeader::unmarshal(&mut reader) {
            Ok(h) => h,
            Err(err) => {
                // Decode error must be silently discarded
                // [RFC6347 Section-4.1.2.7]
                debug!(
                    "{}: discarded broken packet: {}",
                    srv_cli_str(ctx.is_client),
                    err
                );
                return (false, None, None);
            }
        };

        // Validate epoch
        let epoch = ctx.remote_epoch.load(Ordering::Relaxed);
        if h.epoch > epoch {
            if h.epoch > epoch + 1 {
                debug!(
                    "{}: discarded future packet (epoch: {}, seq: {})",
                    srv_cli_str(ctx.is_client),
                    h.epoch,
                    h.sequence_number,
                );
                return (false, None, None);
            }
            if enqueue {
                debug!(
                    "{}: received packet of next epoch, queuing packet",
                    srv_cli_str(ctx.is_client)
                );
                ctx.encrypted_packets.push(pkt);
            }
            return (false, None, None);
        }

        // Anti-replay protection
        while ctx.replay_detector.len() <= h.epoch as usize {
            ctx.replay_detector
                .push(Box::new(SlidingWindowDetector::new(
                    ctx.replay_protection_window,
                    MAX_SEQUENCE_NUMBER,
                )));
        }

        let ok = ctx.replay_detector[h.epoch as usize].check(h.sequence_number);
        if !ok {
            debug!(
                "{}: discarded duplicated packet (epoch: {}, seq: {})",
                srv_cli_str(ctx.is_client),
                h.epoch,
                h.sequence_number,
            );
            return (false, None, None);
        }

        // Decrypt
        if h.epoch != 0 {
            let invalid_cipher_suite = {
                let cipher_suite = ctx.cipher_suite.lock().await;
                if cipher_suite.is_none() {
                    true
                } else if let Some(cipher_suite) = &*cipher_suite {
                    !cipher_suite.is_initialized()
                } else {
                    false
                }
            };
            if invalid_cipher_suite {
                if enqueue {
                    debug!(
                        "{}: handshake not finished, queuing packet",
                        srv_cli_str(ctx.is_client)
                    );
                    ctx.encrypted_packets.push(pkt);
                }
                return (false, None, None);
            }

            let cipher_suite = ctx.cipher_suite.lock().await;
            if let Some(cipher_suite) = &*cipher_suite {
                pkt = match cipher_suite.decrypt(&pkt) {
                    Ok(pkt) => pkt,
                    Err(err) => {
                        debug!("{}: decrypt failed: {}", srv_cli_str(ctx.is_client), err);
                        return (false, None, None);
                    }
                };
            }
        }

        let is_handshake = match ctx.fragment_buffer.push(&pkt) {
            Ok(is_handshake) => is_handshake,
            Err(err) => {
                // Decode error must be silently discarded
                // [RFC6347 Section-4.1.2.7]
                debug!("{}: defragment failed: {}", srv_cli_str(ctx.is_client), err);
                return (false, None, None);
            }
        };
        if is_handshake {
            ctx.replay_detector[h.epoch as usize].accept();
            while let Ok((out, epoch)) = ctx.fragment_buffer.pop() {
                let mut reader = BufReader::new(out.as_slice());
                let raw_handshake = match Handshake::unmarshal(&mut reader) {
                    Ok(rh) => {
                        trace!(
                            "Recv [handshake:{}] -> {} (epoch: {}, seq: {})",
                            srv_cli_str(ctx.is_client),
                            rh.handshake_header.handshake_type.to_string(),
                            h.epoch,
                            rh.handshake_header.message_sequence
                        );
                        rh
                    }
                    Err(err) => {
                        debug!(
                            "{}: handshake parse failed: {}",
                            srv_cli_str(ctx.is_client),
                            err
                        );
                        continue;
                    }
                };

                ctx.cache
                    .push(
                        out,
                        epoch,
                        raw_handshake.handshake_header.message_sequence,
                        raw_handshake.handshake_header.handshake_type,
                        !ctx.is_client,
                    )
                    .await;
            }

            return (true, None, None);
        }

        let mut reader = BufReader::new(pkt.as_slice());
        let r = match RecordLayer::unmarshal(&mut reader) {
            Ok(r) => r,
            Err(err) => {
                return (
                    false,
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::DecodeError,
                    }),
                    Some(err),
                );
            }
        };

        match r.content {
            Content::Alert(mut a) => {
                trace!("{}: <- {}", srv_cli_str(ctx.is_client), a.to_string());
                if a.alert_description == AlertDescription::CloseNotify {
                    // Respond with a close_notify [RFC5246 Section 7.2.1]
                    a = Alert {
                        alert_level: AlertLevel::Warning,
                        alert_description: AlertDescription::CloseNotify,
                    };
                }
                ctx.replay_detector[h.epoch as usize].accept();
                return (
                    false,
                    Some(a),
                    Some(Error::new(format!("Error of Alert {}", a.to_string()))),
                ); //TODO: &errAlert { content });
            }
            Content::ChangeCipherSpec(_) => {
                let invalid_cipher_suite = {
                    let cipher_suite = ctx.cipher_suite.lock().await;
                    if cipher_suite.is_none() {
                        true
                    } else if let Some(cipher_suite) = &*cipher_suite {
                        !cipher_suite.is_initialized()
                    } else {
                        false
                    }
                };

                if invalid_cipher_suite {
                    if enqueue {
                        debug!(
                            "{}: CipherSuite not initialized, queuing packet",
                            srv_cli_str(ctx.is_client)
                        );
                        ctx.encrypted_packets.push(pkt);
                    }
                    return (false, None, None);
                }

                let new_remote_epoch = h.epoch + 1;
                trace!(
                    "{}: <- ChangeCipherSpec (epoch: {})",
                    srv_cli_str(ctx.is_client),
                    new_remote_epoch
                );

                if epoch + 1 == new_remote_epoch {
                    ctx.remote_epoch.store(new_remote_epoch, Ordering::Relaxed);
                    ctx.replay_detector[h.epoch as usize].accept();
                }
            }
            Content::ApplicationData(a) => {
                if h.epoch == 0 {
                    return (
                        false,
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::UnexpectedMessage,
                        }),
                        Some(ERR_APPLICATION_DATA_EPOCH_ZERO.clone()),
                    );
                }

                ctx.replay_detector[h.epoch as usize].accept();

                let _ = ctx.decrypted_tx.send(Ok(a.data)).await;
                //TODO
                /*select {
                    case self.decrypted < - content.data:
                    case < -c.closed.Done():
                }*/
            }
            _ => {
                return (
                    false,
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::UnexpectedMessage,
                    }),
                    Some(ERR_UNHANDLED_CONTEXT_TYPE.clone()),
                );
            }
        };

        (false, None, None)
    }

    fn is_connection_closed(&self) -> bool {
        /*select {
        case <-c.closed.Done():
            return true
        default:
            return false
        }*/
        self.closed
    }

    pub(crate) fn set_local_epoch(&mut self, epoch: u16) {
        self.state.local_epoch.store(epoch, Ordering::Relaxed);
    }

    pub(crate) fn get_local_epoch(&self) -> u16 {
        self.state.local_epoch.load(Ordering::Relaxed)
    }
}

fn compact_raw_packets(raw_packets: &[Vec<u8>], maximum_transmission_unit: usize) -> Vec<Vec<u8>> {
    let mut combined_raw_packets = vec![];
    let mut current_combined_raw_packet = vec![];

    for raw_packet in raw_packets {
        if !current_combined_raw_packet.is_empty()
            && current_combined_raw_packet.len() + raw_packet.len() >= maximum_transmission_unit
        {
            combined_raw_packets.push(current_combined_raw_packet);
            current_combined_raw_packet = vec![];
        }
        current_combined_raw_packet.extend_from_slice(raw_packet);
    }

    combined_raw_packets.push(current_combined_raw_packet);

    combined_raw_packets
}

fn split_bytes(bytes: &[u8], split_len: usize) -> Vec<Vec<u8>> {
    let mut splits = vec![];
    let num_bytes = bytes.len();
    for i in (0..num_bytes).step_by(split_len) {
        let mut j = i + split_len;
        if j > num_bytes {
            j = num_bytes;
        }

        splits.push(bytes[i..j].to_vec());
    }

    splits
}
