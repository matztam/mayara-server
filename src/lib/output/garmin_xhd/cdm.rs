//! Discovery heartbeat of the emulated xHD radar (`239.254.2.2:50050`).
//!
//! Every Garmin marine device announces itself with a CDM "V2 heartbeat", and
//! a plotter only offers a radar it has heard one from. The emulated radar
//! announces itself as a GMR xHD, quickly at first so it shows up on a plotter
//! that was already switched on, then at the leisurely rate a real one uses.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio_graceful_shutdown::SubsystemHandle;

use crate::brand::garmin::protocol::{CDM_HEARTBEAT_ADDRESS, MSG_CDM_HEARTBEAT};
use crate::radar::RadarError;

use super::multicast_send;

/// Product id of the GMR xHD, which is what makes a plotter treat the
/// announcement as coming from a radar it knows how to drive.
const PRODUCT_ID_XHD: u16 = 0x06d0;

/// Remaining identity fields, as a real GMR xHD sends them.
const PRODUCT_SUBTYPE: u8 = 5;
const SYC_GROUP_ID: u8 = 6;
const SERVICE_CLASS: u8 = 1;
const SERVICE_VERSION: u16 = 2;
const SERVICE_ID: u32 = 0x08d4_0aa0;

/// Heartbeats sent at the fast rate before settling into the slow one.
const FAST_HEARTBEATS: u32 = 30;
const FAST_INTERVAL: Duration = Duration::from_secs(1);
const SLOW_INTERVAL: Duration = Duration::from_secs(5);

/// Build the `0x038e` announcement. The layout is documented in
/// `research/garmin/discovery-handshake.md`.
fn heartbeat(sequence: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(26);
    body.push(2); // version marker: V2 heartbeat
    body.push(0); // padding
    body.extend_from_slice(&PRODUCT_ID_XHD.to_le_bytes());
    body.push(0); // simulator mode: a real radar
    body.push(PRODUCT_SUBTYPE);
    body.push(SYC_GROUP_ID);
    body.push(1); // constant
    body.push(1); // one service follows
    body.extend_from_slice(&[0u8; 3]); // padding

    body.push(SERVICE_CLASS);
    body.push(0); // instance
    body.extend_from_slice(&SERVICE_VERSION.to_le_bytes());
    body.extend_from_slice(&SERVICE_ID.to_le_bytes());

    // Serialized tail: one tagged field holding the sequence counter.
    body.push(1); // tag
    body.push(4); // length
    body.extend_from_slice(&sequence.to_le_bytes());

    super::packet(MSG_CDM_HEARTBEAT, &body)
}

pub(super) async fn run(
    local_addr: Ipv4Addr,
    subsys: &mut SubsystemHandle,
) -> Result<(), RadarError> {
    let socket = match multicast_send(&CDM_HEARTBEAT_ADDRESS, local_addr) {
        Ok(socket) => socket,
        Err(e) => {
            log::error!("Garmin xHD heartbeat: cannot open socket: {e}");
            return Ok(());
        }
    };

    let mut sequence: u32 = 0;
    loop {
        if let Err(e) = socket.send(&heartbeat(sequence)).await {
            log::warn!("Garmin xHD heartbeat: send failed: {e}");
        }
        sequence = sequence.wrapping_add(1);

        let interval = if sequence < FAST_HEARTBEATS {
            FAST_INTERVAL
        } else {
            SLOW_INTERVAL
        };

        tokio::select! {
            biased;
            _ = subsys.on_shutdown_requested() => return Ok(()),
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::garmin::discovery;

    #[test]
    fn heartbeat_identifies_the_radar_as_a_gmr_xhd() {
        let packet = heartbeat(7);
        assert_eq!(
            u32::from_le_bytes(packet[0..4].try_into().unwrap()),
            MSG_CDM_HEARTBEAT
        );

        let parsed = discovery::parse(&packet[8..]).expect("valid heartbeat");
        assert_eq!(parsed.product_id, PRODUCT_ID_XHD);
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.simulator_mode, 0);
        assert_eq!(parsed.product_subtype, PRODUCT_SUBTYPE);
        assert_eq!(discovery::product_name(parsed.product_id), Some("GMR xHD"));
    }
}
