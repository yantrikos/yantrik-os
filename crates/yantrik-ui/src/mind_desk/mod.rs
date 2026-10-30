//! What the Mind desk is told (#1023, phase 1): the shell's side of the bars around Mind View.
//!
//! The desk (`apps/mind-desk`) is a window of Mind View, and a Mind acting in there can read it
//! and press what it offers. So the shell does not hand it an agent, it hands it a *projection*:
//! [`project`] reads the Agents store and writes the fields of
//! [`yantrik_ipc_contracts::mind_desk::DeskSnapshot`], which has no place for an argument, a
//! path, a prompt, a command or a token. Every string on it is cleaned by the contract's own
//! functions on the way out.
//!
//! - [`project`]: which Mind the desk shows, and what of it.
//! - [`account`]: which account a harness runs under, from what the kernel said when it attached.
//! - `control_mind_desk` (a sibling module of this one's parent): the `mind_desk` action that
//!   serves the projection to the person's own processes and refuses a Mind's.

pub mod account;
pub mod project;

#[cfg(test)]
mod tests;
