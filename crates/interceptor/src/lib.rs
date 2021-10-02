#![warn(rust_2018_idioms)]
#![allow(dead_code)]

pub mod chain;
pub mod error;
pub mod nack;
pub mod noop;
pub mod registry;
pub mod rr;
pub mod sr;
pub mod stream_info;
pub mod stream_reader;

use stream_info::StreamInfo;

use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Interceptor can be used to add functionality to you PeerConnections by modifying any incoming/outgoing rtp/rtcp
/// packets, or sending your own packets as needed.
#[async_trait]
pub trait Interceptor {
    /// bind_rtcp_reader lets you modify any incoming RTCP packets. It is called once per sender/receiver, however this might
    /// change in the future. The returned method will be called once per packet batch.
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync>;

    /// bind_rtcp_writer lets you modify any outgoing RTCP packets. It is called once per PeerConnection. The returned method
    /// will be called once per packet batch.
    async fn bind_rtcp_writer(
        &self,
        writer: Arc<dyn RTCPWriter + Send + Sync>,
    ) -> Arc<dyn RTCPWriter + Send + Sync>;

    /// bind_local_stream lets you modify any outgoing RTP packets. It is called once for per LocalStream. The returned method
    /// will be called once per rtp packet.
    async fn bind_local_stream(
        &self,
        info: &StreamInfo,
        writer: Arc<dyn RTPWriter + Send + Sync>,
    ) -> Arc<dyn RTPWriter + Send + Sync>;

    /// unbind_local_stream is called when the Stream is removed. It can be used to clean up any data related to that track.
    async fn unbind_local_stream(&self, info: &StreamInfo);

    /// bind_remote_stream lets you modify any incoming RTP packets. It is called once for per RemoteStream. The returned method
    /// will be called once per rtp packet.
    async fn bind_remote_stream(
        &self,
        info: &StreamInfo,
        reader: Arc<dyn RTPReader + Send + Sync>,
    ) -> Arc<dyn RTPReader + Send + Sync>;

    /// unbind_remote_stream is called when the Stream is removed. It can be used to clean up any data related to that track.
    async fn unbind_remote_stream(&self, info: &StreamInfo);

    async fn close(&self) -> Result<()>;
}

/// RTPWriter is used by Interceptor.bind_local_stream.
#[async_trait]
pub trait RTPWriter {
    /// write a rtp packet
    async fn write(&self, pkt: &rtp::packet::Packet, attributes: &Attributes) -> Result<usize>;
}

/// RTPReader is used by Interceptor.bind_remote_stream.
#[async_trait]
pub trait RTPReader {
    /// read a rtp packet
    async fn read(&self, buf: &mut [u8], attributes: &Attributes) -> Result<(usize, Attributes)>;
}

/// RTCPWriter is used by Interceptor.bind_rtcpwriter.
#[async_trait]
pub trait RTCPWriter {
    /// write a batch of rtcp packets
    async fn write(
        &self,
        pkt: &(dyn rtcp::packet::Packet + Send + Sync),
        attributes: &Attributes,
    ) -> Result<usize>;
}

/// RTCPReader is used by Interceptor.bind_rtcpreader.
#[async_trait]
pub trait RTCPReader {
    /// read a batch of rtcp packets
    async fn read(&self, buf: &mut [u8], attributes: &Attributes) -> Result<(usize, Attributes)>;
}

/// Attributes are a generic key/value store used by interceptors
pub type Attributes = HashMap<usize, usize>;
