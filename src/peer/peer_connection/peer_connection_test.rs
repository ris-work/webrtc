use super::*;
use crate::media::Sample;

use crate::api::APIBuilder;
use bytes::Bytes;
use tokio::time::Duration;
use util::vnet::net::{Net, NetConfig};
use util::vnet::router::{Router, RouterConfig};
use waitgroup::WaitGroup;

pub(crate) async fn create_vnet_pair(
) -> Result<(PeerConnection, PeerConnection, Arc<Mutex<Router>>)> {
    // Create a root router
    let wan = Arc::new(Mutex::new(Router::new(RouterConfig {
        cidr: "1.2.3.0/24".to_owned(),
        ..Default::default()
    })?));

    // Create a network interface for offerer
    let offer_vnet = Arc::new(Net::new(Some(NetConfig {
        static_ips: vec!["1.2.3.4".to_owned()],
        ..Default::default()
    })));

    // Add the network interface to the router
    let nic = offer_vnet.get_nic()?;
    {
        let mut w = wan.lock().await;
        w.add_net(Arc::clone(&nic)).await?;
    }
    {
        let n = nic.lock().await;
        n.set_router(Arc::clone(&wan)).await?;
    }

    let mut offer_setting_engine = SettingEngine::default();
    offer_setting_engine.set_vnet(Some(offer_vnet));
    offer_setting_engine.set_ice_timeouts(
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
        Some(Duration::from_millis(200)),
    );

    // Create a network interface for answerer
    let answer_vnet = Arc::new(Net::new(Some(NetConfig {
        static_ips: vec!["1.2.3.5".to_owned()],
        ..Default::default()
    })));

    // Add the network interface to the router
    let nic = answer_vnet.get_nic()?;
    {
        let mut w = wan.lock().await;
        w.add_net(Arc::clone(&nic)).await?;
    }
    {
        let n = nic.lock().await;
        n.set_router(Arc::clone(&wan)).await?;
    }

    let mut answer_setting_engine = SettingEngine::default();
    answer_setting_engine.set_vnet(Some(answer_vnet));
    answer_setting_engine.set_ice_timeouts(
        Some(Duration::from_secs(1)),
        Some(Duration::from_secs(1)),
        Some(Duration::from_millis(200)),
    );

    // Start the virtual network by calling Start() on the root router
    {
        let mut w = wan.lock().await;
        w.start().await?;
    }

    let mut offer_media_engine = MediaEngine::default();
    offer_media_engine.register_default_codecs()?;
    let offer_peer_connection = APIBuilder::new()
        .with_setting_engine(offer_setting_engine)
        .with_media_engine(offer_media_engine)
        .build()
        .new_peer_connection(RTCConfiguration::default())
        .await?;

    let mut answer_media_engine = MediaEngine::default();
    answer_media_engine.register_default_codecs()?;
    let answer_peer_connection = APIBuilder::new()
        .with_setting_engine(answer_setting_engine)
        .with_media_engine(answer_media_engine)
        .build()
        .new_peer_connection(RTCConfiguration::default())
        .await?;

    Ok((offer_peer_connection, answer_peer_connection, wan))
}

/// new_pair creates two new peer connections (an offerer and an answerer)
/// *without* using an api (i.e. using the default settings).
pub(crate) async fn new_pair(api: &API) -> Result<(PeerConnection, PeerConnection)> {
    let pca = api.new_peer_connection(RTCConfiguration::default()).await?;
    let pcb = api.new_peer_connection(RTCConfiguration::default()).await?;

    Ok((pca, pcb))
}

pub(crate) async fn signal_pair(
    pc_offer: &mut PeerConnection,
    pc_answer: &mut PeerConnection,
) -> Result<()> {
    // Note(albrow): We need to create a data channel in order to trigger ICE
    // candidate gathering in the background for the JavaScript/Wasm bindings. If
    // we don't do this, the complete offer including ICE candidates will never be
    // generated.
    pc_offer
        .create_data_channel("initial_data_channel", None)
        .await?;

    let offer = pc_offer.create_offer(None).await?;

    let mut offer_gathering_complete = pc_offer.gathering_complete_promise().await;
    pc_offer.set_local_description(offer).await?;

    let _ = offer_gathering_complete.recv().await;

    pc_answer
        .set_remote_description(
            pc_offer
                .local_description()
                .await
                .ok_or(Error::new("non local description".to_owned()))?,
        )
        .await?;

    let answer = pc_answer.create_answer(None).await?;

    let mut answer_gathering_complete = pc_answer.gathering_complete_promise().await;
    pc_answer.set_local_description(answer).await?;

    let _ = answer_gathering_complete.recv().await;

    pc_offer
        .set_remote_description(
            pc_answer
                .local_description()
                .await
                .ok_or(Error::new("non local description".to_owned()))?,
        )
        .await
}

pub(crate) async fn close_pair_now(pc1: &PeerConnection, pc2: &PeerConnection) {
    let mut fail = false;
    if let Err(err) = pc1.close().await {
        log::error!("Failed to close PeerConnection: {}", err);
        fail = true;
    }
    if let Err(err) = pc2.close().await {
        log::error!("Failed to close PeerConnection: {}", err);
        fail = true;
    }

    assert!(!fail);
}

pub(crate) async fn close_pair(
    pc1: &PeerConnection,
    pc2: &PeerConnection,
    mut done_rx: mpsc::Receiver<()>,
) {
    let timeout = tokio::time::sleep(Duration::from_secs(1));
    tokio::pin!(timeout);

    tokio::select! {
        _ = timeout.as_mut() =>{
            assert!(false, "close_pair timed out waiting for done signal");
        }
        _ = done_rx.recv() =>{
            close_pair_now(pc1, pc2).await;
        }
    }
}

/*
func offerMediaHasDirection(offer SessionDescription, kind RTPCodecType, direction RTPTransceiverDirection) bool {
    parsed := &sdp.SessionDescription{}
    if err := parsed.Unmarshal([]byte(offer.SDP)); err != nil {
        return false
    }

    for _, media := range parsed.MediaDescriptions {
        if media.MediaName.Media == kind.String() {
            _, exists := media.Attribute(direction.String())
            return exists
        }
    }
    return false
}*/

pub(crate) async fn send_video_until_done(
    mut done_rx: mpsc::Receiver<()>,
    tracks: Vec<Arc<TrackLocalStaticSample>>,
    data: Bytes,
) {
    loop {
        let timeout = tokio::time::sleep(Duration::from_millis(20));
        tokio::pin!(timeout);

        tokio::select! {
            _ = timeout.as_mut() =>{
                log::debug!("sendVideoUntilDone timeout");
                for track in &tracks {
                    log::debug!("sendVideoUntilDone track.WriteSample");
                    let result = track.write_sample(&Sample{
                        data: data.clone(),
                        duration: Duration::from_secs(1),
                        ..Default::default()
                    }).await;
                    assert!(result.is_ok());
                }
            }
            _ = done_rx.recv() =>{
                log::debug!("sendVideoUntilDone received done");
                return;
            }
        }
    }
}

pub(crate) async fn until_connection_state(
    pc: &mut PeerConnection,
    wg: &WaitGroup,
    state: RTCPeerConnectionState,
) {
    let w = Arc::new(Mutex::new(Some(wg.worker())));
    pc.on_peer_connection_state_change(Box::new(move |pcs: RTCPeerConnectionState| {
        let w2 = Arc::clone(&w);
        Box::pin(async move {
            if pcs == state {
                let mut worker = w2.lock().await;
                worker.take();
            }
        })
    }))
    .await;
}
