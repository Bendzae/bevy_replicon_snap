use std::collections::VecDeque;
use bevy::prelude::*;
use bevy::reflect::erased_serde::__private::serde::{Deserialize, Serialize};
use bevy_replicon::prelude::*;
use bevy_replicon::replicon_core::replication_rules::{DeserializeFn, RemoveComponentFn, SerializeFn};

pub struct SnapshotInterpolationPlugin {
    pub max_tick_rate: u16,
}

impl Plugin for SnapshotInterpolationPlugin {
    fn build(&self, app: &mut App) {
        app.replicate::<Interpolated>()
            .insert_resource(InterpolationConfig {
                max_tick_rate: self.max_tick_rate,
            });
    }
}

#[derive(Resource)]
pub struct InterpolationConfig {
    pub max_tick_rate: u16,
}

#[derive(Component, Deserialize, Serialize)]
pub struct Interpolated;

pub trait Interpolate {
    fn interpolate(&self, other: Self, t: f32) -> Self;
}

#[derive(Component, Deserialize, Serialize)]
pub struct SnapshotBuffer<T: Component + Interpolate + Clone> {
    buffer: VecDeque<T>,
    time_since_last_snapshot: f32,
}

impl<T: Component + Interpolate + Clone> SnapshotBuffer<T> {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            time_since_last_snapshot: 0.0,
        }
    }
    pub fn insert(&mut self, element: T) {
        if self.buffer.len() > 1 {
            self.buffer.pop_front();
        }
        self.buffer.push_back(element);
        self.time_since_last_snapshot = 0.0;
    }
}

fn snapshot_buffer_init_system<T: Component + Interpolate + Clone>(
    q_interpolated: Query<(Entity, &T), Added<Interpolated>>,
    mut commands: Commands,
) {
    for (e, initial_value) in q_interpolated.iter() {
        let mut buffer = SnapshotBuffer::new();
        buffer.insert(initial_value.clone());
        commands.entity(e).insert(buffer);
    }
}

fn snapshot_interpolation_system<T: Component + Interpolate + Clone>(
    mut q: Query<(Entity, &mut T, &mut SnapshotBuffer<T>), With<Interpolated>>,
    time: Res<Time>,
    config: Res<InterpolationConfig>,
) {
    for (e, mut component, mut snapshot_buffer) in q.iter_mut() {
        let buffer = &snapshot_buffer.buffer;
        let elapsed = snapshot_buffer.time_since_last_snapshot;
        if buffer.len() < 2 {
            continue;
        }

        let tick_duration = 1.0 / (config.max_tick_rate as f32);

        if elapsed > tick_duration + time.delta_seconds() {
            continue;
        }

        let t = (elapsed / tick_duration).clamp(0., 1.);
        *component = buffer[0].interpolate(buffer[1].clone(), t);
        snapshot_buffer.time_since_last_snapshot += time.delta_seconds();
    }
}

pub trait AppInterpolationExt {
    /// TODO: Add docs
    fn replicate_with_interpolation<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveComponentFn,
    ) -> &mut Self
        where
            C: Component + Interpolate + Clone;
}

impl AppInterpolationExt for App {
    fn replicate_with_interpolation<T>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveComponentFn,
    ) -> &mut Self
        where
            T: Component + Interpolate + Clone,
    {
        self.add_systems(
            PreUpdate,
            (
                snapshot_buffer_init_system::<T>,
                snapshot_interpolation_system::<T>,
            )
                .after(ClientSet::Receive)
                .run_if(resource_exists::<RenetClient>()),
        );
        self.replicate_with::<T>(serialize, deserialize, remove)
    }
}

