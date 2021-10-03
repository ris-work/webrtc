pub mod sender_stream;

use crate::*;
use sender_stream::SenderStream;

use crate::error::Error;
use anyhow::Result;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::sync::{mpsc, Mutex};
use waitgroup::WaitGroup;

/// SenderBuilder can be used to configure SenderReport Interceptor.
#[derive(Default)]
pub struct SenderBuilder {
    interval: Option<Duration>,
    now: Option<NowFn>,
}

impl SenderBuilder {
    /// with_interval sets send interval for the interceptor.
    pub fn with_interval(mut self, interval: Duration) -> SenderBuilder {
        self.interval = Some(interval);
        self
    }

    /// with_now_fn sets an alternative for the time.Now function.
    pub fn with_now_fn(mut self, now: NowFn) -> SenderBuilder {
        self.now = Some(now);
        self
    }

    pub fn build(mut self) -> SenderReport {
        let (close_tx, close_rx) = mpsc::channel(1);
        SenderReport {
            internal: Arc::new(SenderReportInternal {
                interval: if let Some(interval) = self.interval.take() {
                    interval
                } else {
                    Duration::from_secs(1)
                },
                now: self.now.take(),
                streams: Mutex::new(HashMap::new()),
                close_rx: Mutex::new(Some(close_rx)),
            }),

            wg: Mutex::new(Some(WaitGroup::new())),
            close_tx: Mutex::new(Some(close_tx)),
        }
    }
}

pub struct SenderReportInternal {
    interval: Duration,
    now: Option<NowFn>,
    streams: Mutex<HashMap<u32, Arc<SenderStream>>>,
    close_rx: Mutex<Option<mpsc::Receiver<()>>>,
}

/// SenderReport interceptor generates sender reports.
pub struct SenderReport {
    internal: Arc<SenderReportInternal>,

    wg: Mutex<Option<WaitGroup>>,
    close_tx: Mutex<Option<mpsc::Sender<()>>>,
}

impl SenderReport {
    /// builder returns a new ReceiverReport builder.
    pub fn builder() -> SenderBuilder {
        SenderBuilder::default()
    }

    async fn is_closed(&self) -> bool {
        let close_tx = self.close_tx.lock().await;
        close_tx.is_none()
    }

    async fn run(
        rtcp_writer: Arc<dyn RTCPWriter + Send + Sync>,
        internal: Arc<SenderReportInternal>,
    ) -> Result<()> {
        let mut ticker = tokio::time::interval(internal.interval);
        let mut close_rx = {
            let mut close_rx = internal.close_rx.lock().await;
            if let Some(close) = close_rx.take() {
                close
            } else {
                return Err(Error::ErrIncorrectReceiverReportCloseRx.into());
            }
        };

        loop {
            tokio::select! {
                _ = ticker.tick() =>{
                    let now = if let Some(f) = &internal.now {
                        f()
                    }else{
                        SystemTime::now()
                    };
                    let streams:Vec<Arc<SenderStream>> = {
                        let m = internal.streams.lock().await;
                        m.values().cloned().collect()
                    };
                    for stream in streams {
                        let pkt = stream.generate_report(now).await;

                        let a = Attributes::new();
                        if let Err(err) = rtcp_writer.write(&pkt, &a).await{
                            log::warn!("failed sending: {}", err);
                        }
                    }
                }
                _ = close_rx.recv() =>{
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl Interceptor for SenderReport {
    /// bind_rtcp_reader lets you modify any incoming RTCP packets. It is called once per sender/receiver, however this might
    /// change in the future. The returned method will be called once per packet batch.
    async fn bind_rtcp_reader(
        &self,
        reader: Arc<dyn RTCPReader + Send + Sync>,
    ) -> Arc<dyn RTCPReader + Send + Sync> {
        reader
    }

    /// bind_rtcp_writer lets you modify any outgoing RTCP packets. It is called once per PeerConnection. The returned method
    /// will be called once per packet batch.
    async fn bind_rtcp_writer(
        &self,
        writer: Arc<dyn RTCPWriter + Send + Sync>,
    ) -> Arc<dyn RTCPWriter + Send + Sync> {
        if self.is_closed().await {
            return writer;
        }

        let mut w = {
            let wait_group = self.wg.lock().await;
            wait_group.as_ref().map(|wg| wg.worker())
        };
        let writer2 = Arc::clone(&writer);
        let internal = Arc::clone(&self.internal);
        tokio::spawn(async move {
            let _d = w.take();
            let _ = SenderReport::run(writer2, internal).await;
        });

        writer
    }

    /// bind_local_stream lets you modify any outgoing RTP packets. It is called once for per LocalStream. The returned method
    /// will be called once per rtp packet.
    async fn bind_local_stream(
        &self,
        info: &StreamInfo,
        writer: Arc<dyn RTPWriter + Send + Sync>,
    ) -> Arc<dyn RTPWriter + Send + Sync> {
        let stream = Arc::new(SenderStream::new(
            info.ssrc,
            info.clock_rate,
            writer,
            self.internal.now.clone(),
        ));
        {
            let mut streams = self.internal.streams.lock().await;
            streams.insert(info.ssrc, Arc::clone(&stream));
        }

        stream
    }

    /// unbind_local_stream is called when the Stream is removed. It can be used to clean up any data related to that track.
    async fn unbind_local_stream(&self, _info: &StreamInfo) {}

    /// bind_remote_stream lets you modify any incoming RTP packets. It is called once for per RemoteStream. The returned method
    /// will be called once per rtp packet.
    async fn bind_remote_stream(
        &self,
        _info: &StreamInfo,
        reader: Arc<dyn RTPReader + Send + Sync>,
    ) -> Arc<dyn RTPReader + Send + Sync> {
        reader
    }

    /// unbind_remote_stream is called when the Stream is removed. It can be used to clean up any data related to that track.
    async fn unbind_remote_stream(&self, _info: &StreamInfo) {}

    /// close closes the Interceptor, cleaning up any data if necessary.
    async fn close(&self) -> Result<()> {
        {
            let mut close_tx = self.close_tx.lock().await;
            close_tx.take();
        }

        {
            let mut wait_group = self.wg.lock().await;
            if let Some(wg) = wait_group.take() {
                wg.wait().await;
            }
        }

        Ok(())
    }
}
