//! The handle a view binds callbacks with, and the window it hands out.

use std::ops::Deref;
use std::rc::{Rc, Weak};

use super::{Core, Inbound};
use crate::app::App;
use crate::queue::Post;
use crate::refused::Refused;

/// The handle a view binds callbacks with. It holds the core weakly: the callbacks live in the
/// window, the window lives in the core, and a strong handle there would keep the core alive
/// for ever. Cloning shares it; it never leaves the UI thread.
pub struct Cx<A: App>(pub(crate) Weak<Core<A>>);

impl<A: App> Clone for Cx<A> {
    fn clone(&self) -> Self {
        Cx(self.0.clone())
    }
}

/// The window, borrowed through a [`Cx`] for as long as the guard lives.
pub struct UiRef<A: App>(Rc<A::Ui>);

impl<A: App> Deref for UiRef<A> {
    type Target = A::Ui;

    fn deref(&self) -> &A::Ui {
        &self.0
    }
}

impl<A: App> Cx<A> {
    pub fn new(core: Rc<Core<A>>) -> Cx<A> {
        Cx(Rc::downgrade(&core))
    }

    pub fn id(&self) -> &'static str {
        A::ID
    }

    /// The core, while it lives. At wire time it always does; a callback fired as the window is
    /// torn down may find it gone, and then does nothing.
    fn core(&self) -> Option<Rc<Core<A>>> {
        self.0.upgrade()
    }

    fn core_or_panic(&self) -> Rc<Core<A>> {
        self.core().expect("the core outlives the window bound to it")
    }

    pub fn run(&self, command: impl Into<A::Command>) {
        match self.core() {
            Some(core) => core.run(command.into()),
            None => tracing::debug!(app = A::ID, "a command after the core was dropped; ignored"),
        }
    }

    pub fn read<R>(&self, f: impl FnOnce(&A::Model) -> R) -> R {
        self.core_or_panic().read(f)
    }

    /// The window, for a view that binds a list model at wire time. Not for `update`.
    pub fn ui(&self) -> UiRef<A> {
        UiRef(self.core_or_panic().ui())
    }

    /// The `Send` endpoint, for tests and for anything off the UI thread.
    pub fn post(&self) -> Post<Inbound<A>> {
        self.core_or_panic().post()
    }

    /// A Slint callback of no arguments that runs the command `f` makes.
    pub fn cmd(&self, f: impl Fn() -> A::Command + 'static) -> impl Fn() + 'static {
        let cx = self.clone();
        move || cx.run(f())
    }

    /// A Slint callback of one argument that runs the command `f` makes.
    pub fn cmd1<T>(&self, f: impl Fn(T) -> A::Command + 'static) -> impl Fn(T) + 'static {
        let cx = self.clone();
        move |t| cx.run(f(t))
    }

    pub fn cmd2<T, U>(&self, f: impl Fn(T, U) -> A::Command + 'static) -> impl Fn(T, U) + 'static {
        let cx = self.clone();
        move |t, u| cx.run(f(t, u))
    }

    /// A Slint callback of one argument whose converter may refuse (a list id that does not
    /// parse): the refusal is the person's and the Mind's, as a command's would be.
    pub fn try_cmd1<T>(
        &self,
        f: impl Fn(T) -> Result<A::Command, Refused> + 'static,
    ) -> impl Fn(T) + 'static {
        let cx = self.clone();
        move |t| match f(t) {
            Ok(command) => cx.run(command),
            Err(refused) => {
                if let Some(core) = cx.core() {
                    core.refuse(refused);
                }
            }
        }
    }
}
