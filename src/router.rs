// src/router.rs
use crate::{
    config::{DataEndpoint, DataType},
    schema::message_meta,
    serialize, validate_packet,
    TelemetryError, TelemetryPacket, Result,
};

// <- pull from alloc so this works under both std and no_std
use alloc::{boxed::Box, sync::Arc, vec::Vec};

/// A local handler bound to a specific endpoint.
pub struct EndpointHandler {
    pub endpoint: DataEndpoint,
    pub handler: Box<dyn Fn(&TelemetryPacket) -> Result<()> + Send + Sync + 'static>,
}

/// Board configuration: which local endpoints exist and how to deliver to them.
#[derive(Default)]
pub struct BoardConfig {
    pub handlers: Vec<EndpointHandler>,
}

impl BoardConfig {
    pub fn new(handlers: Vec<EndpointHandler>) -> Self {
        Self { handlers }
    }
    #[inline]
    fn is_local_endpoint(&self, ep: DataEndpoint) -> bool {
        self.handlers.iter().any(|h| h.endpoint == ep)
    }
}

/// Typed payload the caller can pass to `Router::log`.
pub enum Payload<'a> {
    Bytes(&'a [u8]),
    F32(&'a [f32]),
    F64(&'a [f64]),
    U8(&'a [u8]),
    U16(&'a [u16]),
    U32(&'a [u32]),
    U64(&'a [u64]),
}

impl<'a> Payload<'a> {
    #[inline]
    fn size_bytes(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::F32(s) => 4 * s.len(),
            Self::F64(s) => 8 * s.len(),
            Self::U8(s) => s.len(),
            Self::U16(s) => 2 * s.len(),
            Self::U32(s) => 4 * s.len(),
            Self::U64(s) => 8 * s.len(),
        }
    }

    /// Convert to little-endian bytes (no allocation for `Bytes`).
    fn to_le_vec(&self) -> Vec<u8> {
        match self {
            Self::Bytes(b) => b.to_vec(),
            Self::F32(s) => {
                let mut v = Vec::with_capacity(4 * s.len());
                for x in *s {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
            Self::F64(s) => {
                let mut v = Vec::with_capacity(8 * s.len());
                for x in *s {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
            Self::U8(s) => s.to_vec(),
            Self::U16(s) => {
                let mut v = Vec::with_capacity(2 * s.len());
                for x in *s {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
            Self::U32(s) => {
                let mut v = Vec::with_capacity(4 * s.len());
                for x in *s {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
            Self::U64(s) => {
                let mut v = Vec::with_capacity(8 * s.len());
                for x in *s {
                    v.extend_from_slice(&x.to_le_bytes());
                }
                v
            }
        }
    }
}

/// A small router that can serialize+transmit and/or locally dispatch packets.
pub struct Router {
    transmit: Option<Box<dyn Fn(&[u8]) -> Result<()> + Send + Sync + 'static>>,
    cfg: BoardConfig,
}

impl Router {
    pub fn new<Tx>(transmit: Option<Tx>, cfg: BoardConfig) -> Self
        where
            Tx: Fn(&[u8]) -> Result<()> + Send + Sync + 'static,
    {
        Self {
            transmit: transmit.map(|t| Box::new(t) as _),
            cfg,
        }
    }

    /// Log (send) a packet: serialize once, transmit (if any remote endpoint), then deliver to matching locals.
    fn send(&self, pkt: &TelemetryPacket) -> Result<()> {
        validate_packet(pkt)?;

        // any endpoint not served locally?
        let any_remote = pkt.endpoints.iter().any(|e| !self.cfg.is_local_endpoint(*e));

        // Serialize exactly once.
        let bytes = serialize::serialize_packet(pkt);

        if any_remote {
            if let Some(tx) = &self.transmit {
                tx(&bytes)?;
            }
        }

        for &dest in pkt.endpoints.iter() {
            for h in &self.cfg.handlers {
                if h.endpoint == dest {
                    (h.handler)(pkt)?;
                }
            }
        }
        Ok(())
    }

    /// Accept a serialized buffer (from wire) and locally dispatch to matching endpoints.
    pub fn receive(&self, bytes: &[u8]) -> Result<()> {
        let pkt = serialize::deserialize_packet(bytes)?;
        validate_packet(&pkt)?;
        for &dest in pkt.endpoints.iter() {
            for h in &self.cfg.handlers {
                if h.endpoint == dest {
                    (h.handler)(&pkt)?;
                }
            }
        }
        Ok(())
    }

    /// Build a packet from `ty` + `data` using default endpoints, then send.
    pub fn log(&self, ty: DataType, data: Payload<'_>, timestamp: u64) -> Result<()> {
        let meta = message_meta(ty);

        // size check against schema
        let got = data.size_bytes();
        if got != meta.data_size {
            return Err(TelemetryError::SizeMismatch {
                expected: meta.data_size,
                got,
            });
        }

        let payload = Arc::<[u8]>::from(data.to_le_vec());
        let endpoints = Arc::<[DataEndpoint]>::from(meta.endpoints.to_vec());

        let pkt = TelemetryPacket {
            ty,
            data_size: meta.data_size,
            endpoints,
            timestamp,
            payload,
        };

        self.send(&pkt)
    }

    // convenience overloads
    pub fn log_bytes(&self, ty: DataType, bytes: &[u8], ts: u64) -> Result<()> {
        self.log(ty, Payload::Bytes(bytes), ts)
    }
    pub fn log_f32(&self, ty: DataType, vals: &[f32], ts: u64) -> Result<()> {
        self.log(ty, Payload::F32(vals), ts)
    }
}
