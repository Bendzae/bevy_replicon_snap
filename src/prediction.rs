use std::collections::VecDeque;

use bevy::{
    ecs::{
        component::Component,
        entity::Entity,
        event::Event,
        query::{Added, With, Without},
        system::{Commands, Query, Res, Resource},
    },
    reflect::Reflect,
    time::Time,
};
use bevy_replicon_renet::renet::transport::NetcodeClientTransport;
use serde::{Deserialize, Serialize};
use std::collections::vec_deque::Iter;

use crate::{interpolation::SnapshotBuffer, Interpolate, Interpolated, NetworkOwner};

pub struct EventSnapshot<T: Event> {
    pub value: T,
    pub tick: u32,
    pub delta_time: f32,
}

#[derive(Resource)]
pub struct PredictedEventHistory<T: Event>(pub VecDeque<EventSnapshot<T>>);

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct OwnerPredicted;

#[derive(Component, Reflect)]
pub struct Predicted;

impl<T: Event> PredictedEventHistory<T> {
    pub fn new() -> PredictedEventHistory<T> {
        Self(VecDeque::new())
    }
    pub fn insert(&mut self, value: T, tick: u32, delta_time: f32) -> &mut Self {
        self.0.push_back(EventSnapshot {
            value,
            tick,
            delta_time,
        });
        self
    }
    pub fn remove_stale(&mut self, latest_server_snapshot_tick: u32) -> &mut Self {
        if let Some(last_index) = self
            .0
            .iter()
            .position(|v| v.tick >= latest_server_snapshot_tick)
        {
            self.0.drain(0..last_index);
        } else {
            self.0.clear();
        }
        self
    }

    pub fn predict(&mut self, latest_server_snapshot_tick: u32) -> Iter<'_, EventSnapshot<T>> {
        self.remove_stale(latest_server_snapshot_tick);
        self.0.iter()
    }
}

pub fn owner_prediction_init_system(
    q_owners: Query<(Entity, &NetworkOwner), Added<OwnerPredicted>>,
    client: Res<NetcodeClientTransport>,
    mut commands: Commands,
) {
    let client_id = client.client_id();
    for (e, id) in q_owners.iter() {
        if id.0 == client_id.raw() {
            commands.entity(e).insert(Predicted);
        } else {
            commands.entity(e).insert(Interpolated);
        }
    }
}

/// Advances the snapshot buffer time for predicted entities.
pub fn predicted_snapshot_system<T: Component + Interpolate + Clone>(
    mut q: Query<&mut SnapshotBuffer<T>, (Without<Interpolated>, With<Predicted>)>,
    time: Res<Time>,
) {
    for mut snapshot_buffer in q.iter_mut() {
        snapshot_buffer.time_since_last_snapshot += time.delta_seconds();
    }
}
