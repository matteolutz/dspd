use gpui_kit::{App, Context, Entity};

mod knob;
pub use knob::*;

pub enum MixingComponentValue<T> {
    Entity(Entity<T>),

    DebouncedEntity {
        entity: Entity<T>,

        own_value: T,

        last_change: Option<std::time::Instant>,
        debounce_time: std::time::Duration,
    },
}

impl<T: Clone + 'static> MixingComponentValue<T> {
    pub fn entity(entity: Entity<T>) -> Self {
        Self::Entity(entity)
    }

    pub fn debounced_entity(
        entity: Entity<T>,
        debounce_time: std::time::Duration,
        cx: &App,
    ) -> Self {
        Self::DebouncedEntity {
            own_value: entity.read(cx).clone(),
            entity,
            last_change: None,
            debounce_time,
        }
    }
}

impl<T: 'static> MixingComponentValue<T> {
    pub fn get_entity(&self) -> &Entity<T> {
        match self {
            MixingComponentValue::Entity(e) => e,
            MixingComponentValue::DebouncedEntity { entity, .. } => entity,
        }
    }

    pub fn read<'a>(&'a self, cx: &'a App) -> &'a T {
        match self {
            MixingComponentValue::Entity(e) => e.read(cx),
            MixingComponentValue::DebouncedEntity {
                entity,
                own_value,
                last_change,
                debounce_time,
            } => {
                if let Some(last_change) = last_change {
                    if last_change.elapsed() < *debounce_time {
                        return own_value;
                    }
                }

                entity.read(cx)
            }
        }
    }
}

impl<T: Clone + 'static> MixingComponentValue<T> {
    pub fn update(&mut self, cx: &mut App, update: impl FnOnce(&mut T, &mut Context<T>)) {
        let e = self.get_entity().clone();
        e.update(cx, update);

        if let MixingComponentValue::DebouncedEntity {
            own_value,
            last_change,
            ..
        } = self
        {
            *own_value = e.read(cx).clone();
            *last_change = Some(std::time::Instant::now());
        }
    }
}

impl<T: 'static> From<Entity<T>> for MixingComponentValue<T> {
    fn from(entity: Entity<T>) -> Self {
        Self::Entity(entity)
    }
}
