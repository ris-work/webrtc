/*use crate::api::setting_engine::SettingEngine;
use crate::dtls::dtls_transport::*;

use sctp::association::Association;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

const sctpMaxChannels: u16 = u16::MAX;



pub type OnDataChannelHdlrFn = Box<
    dyn (FnMut(Arc<DataChannel>) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnDataChannelOpenedHdlrFn = Box<
    dyn (FnMut(Arc<DataChannel>) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

/// SCTPTransport provides details about the SCTP transport.
pub struct SCTPTransport {
    dtlsTransport: DTLSTransport,

    // State represents the current state of the SCTP transport.
    state: SCTPTransportState,

    // SCTPTransportState doesn't have an enum to distinguish between New/Connecting
    // so we need a dedicated field
    isStarted: bool,

    // max_message_size represents the maximum size of data that can be passed to
    // DataChannel's send() method.
    maxMessageSize: usize,

    // MaxChannels represents the maximum amount of DataChannel's that can
    // be used simultaneously.
    maxChannels: u16,

    onErrorHandler: Option<OnErrorHdlrFn>,

    sctpAssociation: Association,
    onDataChannelHandler: Option<OnDataChannelHdlrFn>,
    onDataChannelOpenedHandler: Option<OnDataChannelOpenedHdlrFn>,

    // DataChannels
    dataChannels: Vec<Arc<DataChannel>>,
    dataChannelsOpened: u32,
    dataChannelsRequested: u32,
    dataChannelsAccepted: u32,

    setting_engine: SettingEngine,
}
*/
/*
// Transport returns the DTLSTransport instance the SCTPTransport is sending over.
func (r *SCTPTransport) Transport() *DTLSTransport {
    r.lock.RLock()
    defer r.lock.RUnlock()

    return r.dtlsTransport
}

// GetCapabilities returns the SCTPCapabilities of the SCTPTransport.
func (r *SCTPTransport) GetCapabilities() SCTPCapabilities {
    return SCTPCapabilities{
        max_message_size: 0,
    }
}

// Start the SCTPTransport. Since both local and remote parties must mutually
// create an SCTPTransport, SCTP SO (Simultaneous Open) is used to establish
// a connection over SCTP.
func (r *SCTPTransport) Start(remoteCaps SCTPCapabilities) error {
    if r.isStarted {
        return nil
    }
    r.isStarted = true

    dtlsTransport := r.Transport()
    if dtlsTransport == nil || dtlsTransport.conn == nil {
        return errSCTPTransportDTLS
    }

    sctpAssociation, err := sctp.Client(sctp.Config{
        NetConn:       dtlsTransport.conn,
        LoggerFactory: r.api.settingEngine.LoggerFactory,
    })
    if err != nil {
        return err
    }

    r.lock.Lock()
    defer r.lock.Unlock()

    r.sctpAssociation = sctpAssociation
    r.state = SCTPTransportStateConnected

    go r.acceptDataChannels(sctpAssociation)

    return nil
}

// Stop stops the SCTPTransport
func (r *SCTPTransport) Stop() error {
    r.lock.Lock()
    defer r.lock.Unlock()
    if r.sctpAssociation == nil {
        return nil
    }
    err := r.sctpAssociation.Close()
    if err != nil {
        return err
    }

    r.sctpAssociation = nil
    r.state = SCTPTransportStateClosed

    return nil
}

func (r *SCTPTransport) acceptDataChannels(a *sctp.Association) {
    for {
        dc, err := datachannel.Accept(a, &datachannel.Config{
            LoggerFactory: r.api.settingEngine.LoggerFactory,
        })
        if err != nil {
            if err != io.EOF {
                r.log.Errorf("Failed to accept data channel: %v", err)
                r.onError(err)
            }
            return
        }

        var (
            maxRetransmits    *uint16
            maxPacketLifeTime *uint16
        )
        val := uint16(dc.Config.ReliabilityParameter)
        ordered := true

        switch dc.Config.ChannelType {
        case datachannel.ChannelTypeReliable:
            ordered = true
        case datachannel.ChannelTypeReliableUnordered:
            ordered = false
        case datachannel.ChannelTypePartialReliableRexmit:
            ordered = true
            maxRetransmits = &val
        case datachannel.ChannelTypePartialReliableRexmitUnordered:
            ordered = false
            maxRetransmits = &val
        case datachannel.ChannelTypePartialReliableTimed:
            ordered = true
            maxPacketLifeTime = &val
        case datachannel.ChannelTypePartialReliableTimedUnordered:
            ordered = false
            maxPacketLifeTime = &val
        default:
        }

        sid := dc.StreamIdentifier()
        rtcDC, err := r.api.newDataChannel(&DataChannelParameters{
            ID:                &sid,
            Label:             dc.Config.Label,
            Protocol:          dc.Config.Protocol,
            Negotiated:        dc.Config.Negotiated,
            ordered:           ordered,
            max_packet_life_time: maxPacketLifeTime,
            max_retransmits:    maxRetransmits,
        }, r.api.settingEngine.LoggerFactory.NewLogger("ortc"))
        if err != nil {
            r.log.Errorf("Failed to accept data channel: %v", err)
            r.onError(err)
            return
        }

        <-r.onDataChannel(rtcDC)
        rtcDC.handleOpen(dc)

        r.lock.Lock()
        r.dataChannelsOpened++
        handler := r.onDataChannelOpenedHandler
        r.lock.Unlock()

        if handler != nil {
            handler(rtcDC)
        }
    }
}

// OnError sets an event handler which is invoked when
// the SCTP connection error occurs.
func (r *SCTPTransport) OnError(f func(err error)) {
    r.lock.Lock()
    defer r.lock.Unlock()
    r.onErrorHandler = f
}

func (r *SCTPTransport) onError(err error) {
    r.lock.RLock()
    handler := r.onErrorHandler
    r.lock.RUnlock()

    if handler != nil {
        go handler(err)
    }
}

// OnDataChannel sets an event handler which is invoked when a data
// channel message arrives from a remote peer.
func (r *SCTPTransport) OnDataChannel(f func(*DataChannel)) {
    r.lock.Lock()
    defer r.lock.Unlock()
    r.onDataChannelHandler = f
}

// OnDataChannelOpened sets an event handler which is invoked when a data
// channel is opened
func (r *SCTPTransport) OnDataChannelOpened(f func(*DataChannel)) {
    r.lock.Lock()
    defer r.lock.Unlock()
    r.onDataChannelOpenedHandler = f
}

func (r *SCTPTransport) onDataChannel(dc *DataChannel) (done chan struct{}) {
    r.lock.Lock()
    r.dataChannels = append(r.dataChannels, dc)
    r.dataChannelsAccepted++
    handler := r.onDataChannelHandler
    r.lock.Unlock()

    done = make(chan struct{})
    if handler == nil || dc == nil {
        close(done)
        return
    }

    // Run this synchronously to allow setup done in onDataChannelFn()
    // to complete before datachannel event handlers might be called.
    go func() {
        handler(dc)
        close(done)
    }()

    return
}

func (r *SCTPTransport) updateMessageSize() {
    r.lock.Lock()
    defer r.lock.Unlock()

    var remoteMaxMessageSize float64 = 65536 // pion/webrtc#758
    var canSendSize float64 = 65536          // pion/webrtc#758

    r.maxMessageSize = r.calcMessageSize(remoteMaxMessageSize, canSendSize)
}

func (r *SCTPTransport) calcMessageSize(remoteMaxMessageSize, canSendSize float64) float64 {
    switch {
    case remoteMaxMessageSize == 0 &&
        canSendSize == 0:
        return math.Inf(1)

    case remoteMaxMessageSize == 0:
        return canSendSize

    case canSendSize == 0:
        return remoteMaxMessageSize

    case canSendSize > remoteMaxMessageSize:
        return remoteMaxMessageSize

    default:
        return canSendSize
    }
}

func (r *SCTPTransport) updateMaxChannels() {
    val := sctpMaxChannels
    r.maxChannels = &val
}

// MaxChannels is the maximum number of RTCDataChannels that can be open simultaneously.
func (r *SCTPTransport) MaxChannels() uint16 {
    r.lock.Lock()
    defer r.lock.Unlock()

    if r.maxChannels == nil {
        return sctpMaxChannels
    }

    return *r.maxChannels
}

// State returns the current state of the SCTPTransport
func (r *SCTPTransport) State() SCTPTransportState {
    r.lock.RLock()
    defer r.lock.RUnlock()
    return r.state
}

func (r *SCTPTransport) collectStats(collector *statsReportCollector) {
    collector.Collecting()

    stats := TransportStats{
        Timestamp: statsTimestampFrom(time.Now()),
        Type:      StatsTypeTransport,
        ID:        "sctpTransport",
    }

    association := r.association()
    if association != nil {
        stats.BytesSent = association.BytesSent()
        stats.BytesReceived = association.BytesReceived()
    }

    collector.Collect(stats.ID, stats)
}

func (r *SCTPTransport) generateAndSetDataChannelID(dtlsRole DTLSRole, idOut **uint16) error {
    isChannelWithID := func(id uint16) bool {
        for _, d := range r.dataChannels {
            if d.id != nil && *d.id == id {
                return true
            }
        }
        return false
    }

    var id uint16
    if dtlsRole != DTLSRoleClient {
        id++
    }

    max := r.MaxChannels()

    r.lock.Lock()
    defer r.lock.Unlock()
    for ; id < max-1; id += 2 {
        if isChannelWithID(id) {
            continue
        }
        *idOut = &id
        return nil
    }

    return &rtcerr.OperationError{Err: ErrMaxDataChannelID}
}

func (r *SCTPTransport) association() *sctp.Association {
    if r == nil {
        return nil
    }
    r.lock.RLock()
    association := r.sctpAssociation
    r.lock.RUnlock()
    return association
}
*/
