//! Reference adapters + normalizers for non-EVM chains.
//!
//! Each module exposes `XAdapter`, `XNormalizer` and a `pipeline()` helper. To add a chain,
//! copy one of these: raw serde structs, an [`crate::pipeline::Adapter`] that fetches them,
//! and a [`crate::pipeline::Normalizer`] that maps them. The EVM output is free.

pub mod bitcoin;
pub mod solana;
