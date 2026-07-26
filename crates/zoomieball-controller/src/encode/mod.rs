//! The population-facing encode/decode boundary: fill one role pool's input lanes from world
//! state, step it, and decode its output lanes back into motor commands or mailbox/edge-logit
//! publications. `body.rs` and `coach.rs` own one encoder each; `sense.rs` is the receptor/
//! weight/grouping vocabulary both build their retinas from. Re-exported flat here because
//! `backend.rs`'s `act`/`learn` only ever call the two pulse entry points and the shared
//! saturating narrow (`clamp_i64`), never a submodule directly.

mod body;
mod coach;
mod sense;

pub(crate) use body::encode_bodies;
pub(crate) use coach::encode_coaches;
pub(crate) use sense::clamp_i64;
