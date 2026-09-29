//! Seat pipe I/O only. The authority thread receives the owned peer process
//! object and every complete frame before it chooses an admission or receipt.
//! Neither a PID nor the channel identifier is a grant.

use crate::ipc::{PeerProcessHandle, PrivateIpcError, PrivatePipeListener};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};

static NEXT_CHANNEL: AtomicU64 = AtomicU64::new(1);

/// A process-local correlation key minted by native code, never a caller grant.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SeatChannelId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SeatIoAdmission {
    Admit,
    Reject,
}

pub(crate) enum SeatIoReply {
    Frame(Vec<u8>),
    Reject,
}

pub(crate) enum SeatIoEvent {
    PeerConnected {
        channel: SeatChannelId,
        user_sid: String,
        package_sid: String,
        peer_process: PeerProcessHandle,
        reply: SyncSender<SeatIoAdmission>,
    },
    Frame {
        channel: SeatChannelId,
        user_sid: String,
        package_sid: String,
        bytes: Vec<u8>,
        reply: SyncSender<SeatIoReply>,
    },
    PeerDisconnected {
        channel: SeatChannelId,
    },
}

#[derive(Debug)]
pub(crate) enum SeatIoError {
    Transport(PrivateIpcError),
    MissingPeerIdentity,
    ChannelExhausted,
    AuthorityChannelClosed,
}

impl fmt::Display for SeatIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "SEAT_IO_TRANSPORT: {error}"),
            Self::MissingPeerIdentity => write!(f, "SEAT_IO_MISSING_PEER_IDENTITY"),
            Self::ChannelExhausted => write!(f, "SEAT_IO_CHANNEL_EXHAUSTED"),
            Self::AuthorityChannelClosed => write!(f, "SEAT_IO_AUTHORITY_CHANNEL_CLOSED"),
        }
    }
}

impl std::error::Error for SeatIoError {}

/// Run on a seat I/O worker. `accept_app_container` consumes the 0x47 preface
/// and verifies the current user and exact package SID. No frame is read until
/// the authority thread has received the owned process handle and admitted it.
pub(crate) fn run_seat_io(
    listener: PrivatePipeListener,
    events: &SyncSender<SeatIoEvent>,
) -> Result<(), SeatIoError> {
    let mut pipe = listener.accept_app_container().map_err(SeatIoError::Transport)?;
    let user_sid = pipe.peer_sid().to_owned();
    let package_sid = pipe.peer_package_sid().unwrap_or("").to_owned();
    let peer_process = pipe.take_peer_process().ok_or(SeatIoError::MissingPeerIdentity)?;
    if user_sid.is_empty() || package_sid.is_empty() {
        return Err(SeatIoError::MissingPeerIdentity);
    }
    let value = NEXT_CHANNEL.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
        |value| (value != u64::MAX).then_some(value + 1))
        .map_err(|_| SeatIoError::ChannelExhausted)?;
    let channel = SeatChannelId(value);
    let (admission_sender, admission_receiver) = mpsc::sync_channel(1);
    events.send(SeatIoEvent::PeerConnected {
        channel,
        user_sid: user_sid.clone(),
        package_sid: package_sid.clone(),
        peer_process,
        reply: admission_sender,
    }).map_err(|_| SeatIoError::AuthorityChannelClosed)?;

    let result = (|| {
        match admission_receiver.recv().map_err(|_| SeatIoError::AuthorityChannelClosed)? {
            SeatIoAdmission::Admit => {}
            SeatIoAdmission::Reject => return Ok(()),
        }
        loop {
            let bytes = pipe.read_frame().map_err(SeatIoError::Transport)?;
            let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
            events.send(SeatIoEvent::Frame {
                channel,
                user_sid: user_sid.clone(),
                package_sid: package_sid.clone(),
                bytes,
                reply: reply_sender,
            }).map_err(|_| SeatIoError::AuthorityChannelClosed)?;
            match reply_receiver.recv().map_err(|_| SeatIoError::AuthorityChannelClosed)? {
                SeatIoReply::Frame(frame) => pipe.write_frame(&frame).map_err(SeatIoError::Transport)?,
                SeatIoReply::Reject => return Ok(()),
            }
        }
    })();
    let disconnected = events.send(SeatIoEvent::PeerDisconnected { channel });
    match result {
        Err(error) => Err(error),
        Ok(()) => disconnected.map_err(|_| SeatIoError::AuthorityChannelClosed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::AppContainerProfile;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn same_user_without_package_identity_never_reaches_authority_channel() {
        let package_sid = AppContainerProfile::derived_for_test("Gogoke37.SeatIoReject")
            .expect("test package").sid_identity().expect("test package SID");
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let listener = PrivatePipeListener::bind_app_container(
            &format!("seat-io-{}-{nonce}", std::process::id()), &package_sid)
            .expect("seat listener");
        let path = listener.path().to_owned();
        let (events, receiver) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || run_seat_io(listener, &events));
        let mut client = OpenOptions::new().read(true).write(true).open(path)
            .expect("same-user pipe open");
        client.write_all(&[0x47]).expect("transport preface");
        assert!(matches!(worker.join().expect("seat I/O worker"),
            Err(SeatIoError::Transport(PrivateIpcError::PeerIdentityMismatch { .. }))));
        assert!(receiver.try_recv().is_err(), "unverified peer produced an authority event");
    }
}
