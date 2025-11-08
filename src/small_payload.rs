use crate::config::MAX_STACK_PAYLOAD_SIZE;
use alloc::sync::Arc;
use core::{fmt, ops::Deref};


#[derive(Clone)]
pub enum SmallPayload {
    Inline {
        len: u8,
        buf: [u8; MAX_STACK_PAYLOAD_SIZE],
    },
    Heap(Arc<[u8]>),
}

impl SmallPayload {
    #[inline]
    pub fn new(data: &[u8]) -> Self {
        if data.len() <= MAX_STACK_PAYLOAD_SIZE {
            let mut buf = [0u8; MAX_STACK_PAYLOAD_SIZE];
            buf[..data.len()].copy_from_slice(data);
            Self::Inline {
                len: data.len() as u8,
                buf,
            }
        } else {
            Self::Heap(Arc::from(data))
        }
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            SmallPayload::Inline { len, buf } => &buf[..*len as usize],
            SmallPayload::Heap(arc) => arc,
        }
    }

    #[inline(always)]
    pub fn to_arc(&self) -> Arc<[u8]> {
        match self {
            SmallPayload::Inline { len, buf } => Arc::from(&buf[..*len as usize]),
            SmallPayload::Heap(arc) => arc.clone(),
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        match self {
            SmallPayload::Inline { len, .. } => *len as usize,
            SmallPayload::Heap(a) => a.len(),
        }
    }

    #[inline(always)]
    pub fn is_inline(&self) -> bool {
        matches!(self, SmallPayload::Inline { .. })
    }
}

impl fmt::Debug for SmallPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline { len, .. } => write!(f, "SmallPayload::Inline({} bytes)", len),
            Self::Heap(a) => write!(f, "SmallPayload::Heap({} bytes)", a.len()),
        }
    }
}

impl Deref for SmallPayload {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}
