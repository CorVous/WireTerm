//! Single-owner host bridge between frame producers and the display bridge.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
};

use thiserror::Error;

use crate::{
    frame::PanelFrame,
    transport::{self, DeviceInfo, TransferStage},
};

enum HostCommand {
    DiscoverDevices,
    SendFrame {
        port_name: String,
        frame: Arc<PanelFrame>,
    },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum HostEvent {
    DevicesChanged(Vec<DeviceInfo>),
    DeviceDiscoveryFailed(String),
    TransferProgress(TransferStage),
    TransferFailed(String),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum HostBridgeError {
    #[error("another frame transfer is already active")]
    TransferBusy,
    #[error("the host bridge worker is unavailable")]
    WorkerUnavailable,
    #[error("the host bridge command queue is full")]
    CommandQueueFull,
}

/// Owns the sole serial worker used by the host process.
///
/// The GUI and future playback host submit display-ready [`PanelFrame`] values
/// here. Serial discovery, protocol state, CRC framing, and transfer
/// serialization remain behind this boundary.
pub struct HostBridge {
    commands: SyncSender<HostCommand>,
    events: Receiver<HostEvent>,
    discovery_active: Arc<AtomicBool>,
    transfer_active: Arc<AtomicBool>,
}

impl fmt::Debug for HostBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostBridge")
            .field("transfer_active", &self.is_transfer_active())
            .finish_non_exhaustive()
    }
}

impl HostBridge {
    #[must_use]
    pub fn new() -> Self {
        let (command_sender, command_receiver) = mpsc::sync_channel(4);
        let (event_sender, event_receiver) = mpsc::channel();
        let discovery_active = Arc::new(AtomicBool::new(false));
        let transfer_active = Arc::new(AtomicBool::new(false));
        let worker_discovery_active = Arc::clone(&discovery_active);
        let worker_active = Arc::clone(&transfer_active);
        thread::Builder::new()
            .name("wireterm-host-bridge".to_owned())
            .spawn(move || {
                while let Ok(command) = command_receiver.recv() {
                    match command {
                        HostCommand::DiscoverDevices => {
                            match transport::discover_devices() {
                                Ok(devices) => {
                                    let _ = event_sender.send(HostEvent::DevicesChanged(devices));
                                }
                                Err(error) => {
                                    let _ = event_sender
                                        .send(HostEvent::DeviceDiscoveryFailed(error.to_string()));
                                }
                            }
                            worker_discovery_active.store(false, Ordering::Release);
                        }
                        HostCommand::SendFrame { port_name, frame } => {
                            let result = transport::send_frame(&port_name, &frame, |stage| {
                                let _ = event_sender.send(HostEvent::TransferProgress(stage));
                            });
                            if let Err(error) = result {
                                let _ =
                                    event_sender.send(HostEvent::TransferFailed(error.to_string()));
                            }
                            worker_active.store(false, Ordering::Release);
                        }
                        HostCommand::Shutdown => break,
                    }
                }
                worker_discovery_active.store(false, Ordering::Release);
                worker_active.store(false, Ordering::Release);
            })
            .expect("host bridge worker should start");

        let bridge = Self {
            commands: command_sender,
            events: event_receiver,
            discovery_active,
            transfer_active,
        };
        let _ = bridge.refresh_devices();
        bridge
    }

    pub fn refresh_devices(&self) -> Result<(), HostBridgeError> {
        if self
            .discovery_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        if let Err(error) = self.try_send_command(HostCommand::DiscoverDevices) {
            self.discovery_active.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    pub fn send_frame(
        &self,
        port_name: String,
        frame: Arc<PanelFrame>,
    ) -> Result<(), HostBridgeError> {
        if self
            .transfer_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(HostBridgeError::TransferBusy);
        }
        if let Err(error) = self.try_send_command(HostCommand::SendFrame { port_name, frame }) {
            self.transfer_active.store(false, Ordering::Release);
            return Err(error);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_transfer_active(&self) -> bool {
        self.transfer_active.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn try_recv(&self) -> Option<HostEvent> {
        self.events.try_recv().ok()
    }

    fn try_send_command(&self, command: HostCommand) -> Result<(), HostBridgeError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => HostBridgeError::CommandQueueFull,
                TrySendError::Disconnected(_) => HostBridgeError::WorkerUnavailable,
            })
    }
}

impl Default for HostBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HostBridge {
    fn drop(&mut self) {
        let _ = self.commands.try_send(HostCommand::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use crate::frame::{PIXEL_COUNT, PanelColor};

    use super::*;

    #[test]
    fn bridge_serializes_transfer_submissions() {
        let bridge = HostBridge::new();
        let frame = Arc::new(
            PanelFrame::from_palette_pixels(&vec![PanelColor::White; PIXEL_COUNT])
                .expect("valid frame"),
        );

        bridge
            .transfer_active
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            bridge.send_frame("COM1".to_owned(), frame),
            Err(HostBridgeError::TransferBusy)
        );
    }

    #[test]
    fn bridge_coalesces_discovery_requests_while_one_is_active() {
        let bridge = HostBridge::new();
        bridge.discovery_active.store(true, Ordering::Release);

        assert_eq!(bridge.refresh_devices(), Ok(()));
        assert!(bridge.discovery_active.load(Ordering::Acquire));
    }
}
