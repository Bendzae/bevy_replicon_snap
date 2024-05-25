use std::collections::VecDeque;

use bevy::{
    ecs::{
        component::Component,
        query::{With, Without},
        system::{Query, Res, Resource},
    },
    reflect::Reflect,
    time::Time,
};
use serde::{Deserialize, Serialize};

use crate::prediction::Predicted;

pub trait Interpolate {
    fn interpolate(&self, other: Self, t: f32) -> Self;
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct Interpolated;

#[derive(Deserialize, Serialize, Reflect)]
pub struct Snapshot<T: Component + Interpolate + Clone> {
    tick: u32,
    value: T,
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct SnapshotBuffer<T: Component + Interpolate + Clone> {
    pub buffer: VecDeque<T>,
    pub time_since_last_snapshot: f32,
    pub latest_snapshot_tick: u32,
}

#[derive(Resource, Serialize, Deserialize, Debug)]
pub struct SnapshotInterpolationConfig {
    pub max_tick_rate: u16,
}

impl<T: Component + Interpolate + Clone> SnapshotBuffer<T> {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            time_since_last_snapshot: 0.0,
            latest_snapshot_tick: 0,
        }
    }
    pub fn insert(&mut self, element: T, tick: u32) {
        if self.buffer.len() > 1 {
            self.buffer.pop_front();
        }
        self.buffer.push_back(element);
        self.time_since_last_snapshot = 0.0;
        self.latest_snapshot_tick = tick;
    }

    pub fn latest_snapshot(&self) -> T {
        self.buffer.iter().last().unwrap().clone()
    }

    pub fn latest_snapshot_tick(&self) -> u32 {
        self.latest_snapshot_tick
    }

    pub fn age(&self) -> f32 {
        self.time_since_last_snapshot
    }
}

/// Interpolate between snapshots.
pub fn snapshot_interpolation_system<T: Component + Interpolate + Clone>(
    mut q: Query<(&mut T, &mut SnapshotBuffer<T>), (With<Interpolated>, Without<Predicted>)>,
    time: Res<Time>,
    config: Res<SnapshotInterpolationConfig>,
) {
    for (mut component, mut snapshot_buffer) in q.iter_mut() {
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
