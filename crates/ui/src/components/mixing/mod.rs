mod knob;
use gpui_kit::{App, Context, Entity, Subscription};
pub use knob::*;

const MIXING_COMPONENT_RELEASE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

pub struct MixingComponentValue<T> {
    local: Option<T>,
    remote: T,

    /// Whether the component is currently updating its local value
    /// i.e. the user is interacting with the component and changing its value
    is_updating: bool,

    /// The deadline for releasing the local value and reverting to the remote value
    release_deadline: Option<std::time::Instant>,

    _subs: Vec<Subscription>,
}

impl<T: 'static + Clone> MixingComponentValue<T> {
    pub fn from_entity(entity: &Entity<T>, cx: &mut Context<Self>) -> Self {
        let remote = entity.read(cx).clone();

        let _subs = vec![cx.observe(entity, |this, e, cx| {
            this.handle_remote_update(e.read(cx).clone(), cx)
        })];

        Self {
            local: None,
            remote,
            is_updating: false,
            release_deadline: None,
            _subs,
        }
    }

    pub fn from_entity_with<E, F>(entity: &Entity<E>, with: F, cx: &mut Context<Self>) -> Self
    where
        E: 'static,
        F: Fn(&E) -> T + 'static,
    {
        let remote = with(entity.read(cx));

        let _subs = vec![cx.observe(entity, move |this, e, cx| {
            this.handle_remote_update(with(&e.read(cx)), cx)
        })];

        Self {
            local: None,
            remote,
            is_updating: false,
            release_deadline: None,
            _subs,
        }
    }
}

impl<T: 'static> MixingComponentValue<T> {
    pub fn display_value(&self) -> &T {
        self.local.as_ref().unwrap_or(&self.remote)
    }

    pub fn start_updating(&mut self) {
        self.is_updating = true;
        self.release_deadline = None;
    }

    pub fn set_local_value(&mut self, value: T, cx: &mut Context<Self>) {
        self.local = Some(value);
        cx.notify();
    }

    pub fn update_local_value<R>(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut T, &mut App) -> R,
    ) -> R
    where
        T: Clone,
    {
        let current_value = self.local.get_or_insert_with(|| self.remote.clone());
        let result = update(current_value, cx);
        cx.notify();
        result
    }

    pub fn stop_updating(&mut self) {
        self.is_updating = false;
        self.release_deadline = Some(std::time::Instant::now() + MIXING_COMPONENT_RELEASE_DELAY);
    }
}

impl<T: 'static> MixingComponentValue<T> {
    fn handle_remote_update(&mut self, new_remote_value: T, cx: &mut Context<Self>) {
        self.remote = new_remote_value;

        if self.is_updating {
            // If the component is currently updating, we don't want to override the local value
            return;
        }

        let timed_out = self
            .release_deadline
            .is_some_and(|d| std::time::Instant::now() >= d);

        if timed_out {
            self.release_deadline = None;
            self.local = None;
            cx.notify();
        }
    }
}
