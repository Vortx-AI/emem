//! Attester / responder key types.
//!
//! ed25519. Keys are versioned by epoch (spec §7); revocation is by publishing
//! `revoked_at` against an epoch in `/.well-known/emem.json`.

use serde::{Deserialize, Serialize};

/// 32-byte ed25519 public key, base32-rendered for wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttesterKey(pub [u8; 32]);

/// Key rotation epoch. Monotonically increasing per attester.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyEpoch(pub u32);

/// 64-byte ed25519 signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

impl Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // 64 > the largest array size for which serde derives Serialize
        // automatically; emit as a tuple so JSON/CBOR round-trip both work.
        use serde::ser::SerializeTuple;
        let mut t = ser.serialize_tuple(64)?;
        for b in &self.0 {
            t.serialize_element(b)?;
        }
        t.end()
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Signature;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a 64-byte ed25519 signature")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Signature, A::Error> {
                let mut out = [0u8; 64];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| serde::de::Error::invalid_length(i, &self))?;
                }
                Ok(Signature(out))
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Signature, E> {
                if v.len() != 64 {
                    return Err(E::invalid_length(v.len(), &self));
                }
                let mut out = [0u8; 64];
                out.copy_from_slice(v);
                Ok(Signature(out))
            }
        }
        de.deserialize_tuple(64, V)
    }
}
