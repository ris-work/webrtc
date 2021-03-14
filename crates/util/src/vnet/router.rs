use crate::vnet::chunk::Chunk;
use crate::vnet::chunk_queue::ChunkQueue;
use crate::vnet::errors::*;
use crate::vnet::nat::*;
use crate::Error;

use ifaces::*;
use ipnet::*;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::vnet::net::LO0_STR;
use crate::vnet::resolver::Resolver;
use async_trait::async_trait;
use std::str::FromStr;
use std::sync::Arc;

const DEFAULT_ROUTER_QUEUE_SIZE: usize = 0; // unlimited

lazy_static! {
    pub static ref ROUTER_ID_CTR: AtomicU64 = AtomicU64::new(0);
}

// Generate a unique router name
fn assign_router_name() -> String {
    let n = ROUTER_ID_CTR.fetch_add(1, Ordering::SeqCst);
    format!("router{}", n)
}

// RouterConfig ...
#[derive(Default)]
pub struct RouterConfig {
    // name of router. If not specified, a unique name will be assigned.
    name: String,
    // cidr notation, like "192.0.2.0/24"
    cidr: String,
    // static_ips is an array of static IP addresses to be assigned for this router.
    // If no static IP address is given, the router will automatically assign
    // an IP address.
    // This will be ignored if this router is the root.
    static_ips: Vec<String>,
    // static_ip is deprecated. Use static_ips.
    static_ip: String,
    // Internal queue size
    queue_size: usize,
    // Effective only when this router has a parent router
    nat_type: NATType,
    // Minimum Delay
    min_delay: Duration,
    // Max Jitter
    max_jitter: Duration,
}

// NIC is a network interface controller that interfaces Router
#[async_trait]
pub trait NIC {
    fn get_interface(&self, if_name: &str) -> Option<&Interface>;
    async fn on_inbound_chunk(&self, c: &(dyn Chunk + Send + Sync));
    fn get_static_ips(&self) -> &[IpAddr];
    fn set_router(&mut self, r: Arc<Router>) -> Result<(), Error>;
}

// ChunkFilter is a handler users can add to filter chunks.
// If the filter returns false, the packet will be dropped.
pub type ChunkFilterFn = fn(c: &dyn Chunk) -> bool;

// Router ...
#[derive(Default)]
pub struct Router {
    name: String,                              // read-only
    interfaces: Vec<Interface>,                // read-only
    ipv4net: IpNet,                            // read-only
    static_ips: Vec<IpAddr>,                   // read-only
    static_local_ips: HashMap<String, IpAddr>, // read-only,
    last_id: u8, // requires mutex [x], used to assign the last digit of IPv4 address
    queue: ChunkQueue, // read-only
    parent: Option<Arc<Router>>, // read-only
    children: Vec<Arc<Router>>, // read-only
    nat_type: NATType, // read-only
    nat: NetworkAddressTranslator, // read-only
    nics: HashMap<String, Arc<dyn NIC>>, // read-only
    done: Option<mpsc::Sender<()>>, // requires mutex [x]
    resolver: Resolver, // read-only
    chunk_filters: Vec<ChunkFilterFn>, // requires mutex [x]
    min_delay: Duration, // requires mutex [x]
    max_jitter: Duration, // requires mutex [x]
    push_ch: Option<mpsc::Sender<()>>, // writer requires mutex
}

//TODO: remove unsafe
unsafe impl Send for Router {}
unsafe impl Sync for Router {}

#[async_trait]
impl NIC for Router {
    fn get_interface(&self, ifc_name: &str) -> Option<&Interface> {
        for ifc in &self.interfaces {
            if ifc.name == ifc_name {
                return Some(ifc);
            }
        }
        None
    }

    async fn on_inbound_chunk(&self, c: &(dyn Chunk + Send + Sync)) {
        let from_parent: Box<dyn Chunk + Send + Sync> = match self.nat.translate_inbound(c).await {
            Ok(from) => {
                if let Some(from) = from {
                    from
                } else {
                    return;
                }
            }
            Err(err) => {
                log::warn!("[{}] {}", self.name, err);
                return;
            }
        };

        self.push(from_parent).await;
    }

    fn get_static_ips(&self) -> &[IpAddr] {
        &self.static_ips
    }

    // caller must hold the mutex
    fn set_router(&mut self, _parent: Arc<Router>) -> Result<(), Error> {
        /*r.parent = parent
        r.resolver.setParent(parent.resolver)

        // when this method is called, one or more IP address has already been assigned by
        // the parent router.
        ifc, err := r.get_interface("eth0")
        if err != nil {
            return err
        }

        if len(ifc.addrs) == 0 {
            return errNoIPAddrEth0
        }

        mappedIPs := []net.IP{}
        localIPs := []net.IP{}

        for _, ifcAddr := range ifc.addrs {
            var ip net.IP
            switch addr := ifcAddr.(type) {
            case *net.IPNet:
                ip = addr.IP
            case *net.IPAddr: // Do we really need this case?
                ip = addr.IP
            default:
            }

            if ip == nil {
                continue
            }

            mappedIPs = append(mappedIPs, ip)

            if locIP := r.static_local_ips[ip.String()]; locIP != nil {
                localIPs = append(localIPs, locIP)
            }
        }

        // Set up NAT here
        if r.nat_type == nil {
            r.nat_type = &nattype{
                MappingBehavior:   EndpointIndependent,
                FilteringBehavior: EndpointAddrPortDependent,
                Hairpining:        false,
                PortPreservation:  false,
                MappingLifeTime:   30 * time.Second,
            }
        }
        r.nat, err = newNAT(&natConfig{
            name:          r.name,
            nat_type:       *r.nat_type,
            mappedIPs:     mappedIPs,
            localIPs:      localIPs,
            loggerFactory: r.loggerFactory,
        })
        if err != nil {
            return err
        }
        */
        Ok(())
    }
}

impl Router {
    pub fn new(config: RouterConfig) -> Result<Self, Error> {
        let ipv4net: IpNet = config.cidr.parse()?;

        let queue_size = if config.queue_size > 0 {
            config.queue_size
        } else {
            DEFAULT_ROUTER_QUEUE_SIZE
        };

        // set up network interface, lo0
        let lo0 = Interface {
            name: LO0_STR.to_owned(),
            kind: Kind::Ipv4,
            addr: Some(SocketAddr::from_str("127.0.0.1")?),
            mask: None,
            hop: None,
        };

        // set up network interface, eth0
        let eth0 = Interface {
            name: "eth0".to_owned(),
            kind: Kind::Ipv4,
            addr: None,
            mask: None,
            hop: None,
        };

        // local host name resolver
        let resolver = Resolver::new();

        let name = if config.name.is_empty() {
            assign_router_name()
        } else {
            config.name.clone()
        };

        let mut static_ips = vec![];
        let mut static_local_ips = HashMap::new();
        for ip_str in &config.static_ips {
            let ip_pair: Vec<&str> = ip_str.split('/').collect();
            if let Ok(ip) = IpAddr::from_str(ip_pair[0]) {
                if ip_pair.len() > 1 {
                    let loc_ip = IpAddr::from_str(ip_pair[1])?;
                    if !ipv4net.contains(&loc_ip) {
                        return Err(ERR_LOCAL_IP_BEYOND_STATIC_IPS_SUBSET.to_owned());
                    }
                    static_local_ips.insert(ip.to_string(), loc_ip);
                }
                static_ips.push(ip);
            }
        }
        if !config.static_ip.is_empty() {
            log::warn!("static_ip is deprecated. Use static_ips instead");
            if let Ok(ip) = IpAddr::from_str(&config.static_ip) {
                static_ips.push(ip);
            }
        }

        let n_static_local = static_local_ips.len();
        if n_static_local > 0 && n_static_local != static_ips.len() {
            return Err(ERR_LOCAL_IP_NO_STATICS_IPS_ASSOCIATED.to_owned());
        }

        Ok(Router {
            name,
            interfaces: vec![lo0, eth0],
            ipv4net,
            static_ips,
            static_local_ips,
            queue: ChunkQueue::new(queue_size),
            nat_type: config.nat_type,
            nics: HashMap::new(),
            resolver,
            min_delay: config.min_delay,
            max_jitter: config.max_jitter,
            ..Default::default()
        })
    }

    // caller must hold the mutex
    pub(crate) fn get_interfaces(&self) -> &[Interface] {
        &self.interfaces
    }

    // Start ...
    pub async fn start(&mut self) -> Result<(), Error> {
        if self.done.is_some() {
            return Err(ERR_ROUTER_ALREADY_STARTED.to_owned());
        }

        let (done_tx, mut done_rx) = mpsc::channel(1);
        let (push_ch_tx, mut push_ch_rx) = mpsc::channel(1);
        self.done = Some(done_tx);
        self.push_ch = Some(push_ch_tx);

        tokio::spawn(async move {
            while let Ok(d) = Router::process_chunks() {
                if d == Duration::from_secs(0) {
                    tokio::select! {
                     _ = push_ch_rx.recv() =>{},
                     _ = done_rx.recv() => break,
                    }
                } else {
                    let t = tokio::time::sleep(d);
                    tokio::pin!(t);

                    tokio::select! {
                    _ = t.as_mut() => {},
                    _ = done_rx.recv() => break,
                    }
                }
            }
        });

        for _child in &self.children {
            //TODO: let mut c = child.lock().await;
            // c.start().await?;
        }

        Ok(())
    }

    // Stop ...
    pub async fn stop(&mut self) -> Result<(), Error> {
        if self.done.is_none() {
            return Err(ERR_ROUTER_ALREADY_STOPPED.to_owned());
        }

        for _child in &self.children {
            //TODO: let mut c = c.lock().await;
            // c.stop().await?;
        }

        self.push_ch.take();
        self.done.take();

        Ok(())
    }

    // caller must hold the mutex
    pub(crate) fn add_nic(&mut self, nic: Arc<dyn NIC>) -> Result<(), Error> {
        let _ifc = nic.get_interface("eth0");

        let mut ips = nic.get_static_ips().to_vec();
        if ips.is_empty() {
            // assign an IP address
            let ip = self.assign_ip_address()?;
            ips.push(ip);
        }

        //TODO: nic.set_router(r)?;

        for ip in &ips {
            if !self.ipv4net.contains(ip) {
                return Err(ERR_STATIC_IP_IS_BEYOND_SUBNET.to_owned());
            }

            /*TODO: ifc.AddAddr(&net.IPNet{
                IP:   ip,
                Mask: r.ipv4net.Mask,
            })*/

            self.nics.insert(ip.to_string(), Arc::clone(&nic));
        }

        Ok(())
    }

    // AddRouter adds a chile Router.
    pub fn add_router(&mut self, router: Arc<Router>) -> Result<(), Error> {
        //r.mutex.Lock()
        //defer r.mutex.Unlock()

        // Router is a NIC. Add it as a NIC so that packets are routed to this child
        // router.
        let router2 = Arc::clone(&router);

        self.add_nic(router as Arc<dyn NIC>)?;

        //TODO: router.set_router(r)?;

        self.children.push(router2);

        Ok(())
    }

    // AddNet ...
    pub fn add_net(&mut self, nic: Arc<dyn NIC>) -> Result<(), Error> {
        //r.mutex.Lock()
        //defer r.mutex.Unlock()
        self.add_nic(nic)
    }

    // AddHost adds a mapping of hostname and an IP address to the local resolver.
    pub fn add_host(&mut self, host_name: String, ip_addr: String) -> Result<(), Error> {
        self.resolver.add_host(host_name, ip_addr)
    }

    // AddChunkFilter adds a filter for chunks traversing this router.
    // You may add more than one filter. The filters are called in the order of this method call.
    // If a chunk is dropped by a filter, subsequent filter will not receive the chunk.
    pub fn add_chunk_filter(&mut self, filter: ChunkFilterFn) {
        //r.mutex.Lock()
        //defer r.mutex.Unlock()

        self.chunk_filters.push(filter);
    }

    // caller should hold the mutex
    fn assign_ip_address(&mut self) -> Result<IpAddr, Error> {
        // See: https://stackoverflow.com/questions/14915188/ip-address-ending-with-zero

        if self.last_id == 0xfe {
            return Err(ERR_ADDRESS_SPACE_EXHAUSTED.to_owned());
        }

        self.last_id += 1;
        match self.ipv4net.addr() {
            IpAddr::V4(ipv4) => {
                let mut ip = ipv4.octets();
                ip[3] += 1;
                Ok(IpAddr::V4(Ipv4Addr::from(ip)))
            }
            IpAddr::V6(ipv6) => {
                let mut ip = ipv6.octets();
                ip[15] += 1;
                Ok(IpAddr::V6(Ipv6Addr::from(ip)))
            }
        }
    }

    async fn push(&self, mut c: Box<dyn Chunk + Send + Sync>) {
        log::debug!("[{}] route {}", self.name, c);
        if self.done.is_some() {
            c.set_timestamp();
            if self.queue.push(c).await {
                if let Some(push_ch) = &self.push_ch {
                    let _ = push_ch.try_send(());
                }
            } else {
                log::warn!("[{}] queue was full. dropped a chunk", self.name);
            }
        }
    }

    fn process_chunks() -> Result<Duration, Error> {
        //TODO:r.mutex.Lock()
        //defer r.mutex.Unlock()
        /*
        // Introduce jitter by delaying the processing of chunks.
        if r.max_jitter > 0 {
            jitter := time.Duration(rand.Int63n(int64(r.max_jitter))) //nolint:gosec
            time.Sleep(jitter)
        }

        //      cutOff
        //         v min delay
        //         |<--->|
        //  +------------:--
        //  |OOOOOOXXXXX :   --> time
        //  +------------:--
        //  |<--->|     now
        //    due

        enteredAt := time.Now()
        cutOff := enteredAt.Add(-r.min_delay)

        var d time.Duration // the next sleep duration

        for {
            d = 0

            c := r.queue.peek()
            if c == nil {
                break // no more chunk in the queue
            }

            // check timestamp to find if the chunk is due
            if c.getTimestamp().After(cutOff) {
                // There is one or more chunk in the queue but none of them are due.
                // Calculate the next sleep duration here.
                nextExpire := c.getTimestamp().Add(r.min_delay)
                d = nextExpire.Sub(enteredAt)
                break
            }

            var ok bool
            if c, ok = r.queue.pop(); !ok {
                break // no more chunk in the queue
            }

            blocked := false
            for i := 0; i < len(r.chunk_filters); i++ {
                filter := r.chunk_filters[i]
                if !filter(c) {
                    blocked = true
                    break
                }
            }
            if blocked {
                continue // discard
            }

            dstIP := c.getDestinationIP()

            // check if the desination is in our subnet
            if r.ipv4net.Contains(dstIP) {
                // search for the destination NIC
                var nic NIC
                if nic, ok = r.nics[dstIP.String()]; !ok {
                    // NIC not found. drop it.
                    r.log.Debugf("[%s] %s unreachable", r.name, c.String())
                    continue
                }

                // found the NIC, forward the chunk to the NIC.
                // call to NIC must unlock mutex
                r.mutex.Unlock()
                nic.on_inbound_chunk(c)
                r.mutex.Lock()
                continue
            }

            // the destination is outside of this subnet
            // is this WAN?
            if r.parent == nil {
                // this WAN. No route for this chunk
                r.log.Debugf("[%s] no route found for %s", r.name, c.String())
                continue
            }

            // Pass it to the parent via NAT
            toParent, err := r.nat.translateOutbound(c)
            if err != nil {
                return 0, err
            }

            if toParent == nil {
                continue
            }

            //nolint:godox
            /* FIXME: this implementation would introduce a duplicate packet!
            if r.nat.nat_type.Hairpining {
                hairpinned, err := r.nat.translateInbound(toParent)
                if err != nil {
                    r.log.Warnf("[%s] %s", r.name, err.Error())
                } else {
                    go func() {
                        r.push(hairpinned)
                    }()
                }
            }
            */

            // call to parent router mutex unlock mutex
            r.mutex.Unlock()
            r.parent.push(toParent)
            r.mutex.Lock()
        }

        return d, nil*/
        let a = true;
        if a {
            Ok(Duration::from_secs(0))
        } else {
            Err(ERR_NOT_FOUND.to_owned())
        }
    }
}
