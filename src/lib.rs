use std::collections::vec_deque::Iter;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::io::Cursor;

use bevy::prelude::*;
use bevy_replicon::bincode;
use bevy_replicon::bincode::deserialize_from;
use bevy_replicon::client::client_mapper::ServerEntityMap;
use bevy_replicon::client::ServerEntityTicks;
use bevy_replicon::core::replication_rules;
use bevy_replicon::core::replication_rules::{
    serialize_component, DeserializeFn, RemoveComponentFn, SerializeFn,
};
use bevy_replicon::core::replicon_channels::RepliconChannel;
use bevy_replicon::core::replicon_tick::RepliconTick;
use bevy_replicon::prelude::*;
use bevy_replicon_renet::renet::{transport::NetcodeClientTransport, RenetClient};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use bevy_replicon_snap_macros;

#[cfg(feature = "bevy_types")]
pub mod bevy_types;

pub struct SnapshotInterpolationPlugin {
    /// Should reflect the server max tick rate
    pub max_tick_rate: u16,
}

/// Sets for interpolation systems.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum InterpolationSet {
    /// Systems that initializes buffers and flag components for replicated entities.
    ///
    /// Runs in `PreUpdate`.
    Init,
    /// Systems that calculating interpolation.
    ///
    /// Runs in `PreUpdate`.
    Interpolate,
}

impl Plugin for SnapshotInterpolationPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Interpolated>()
            .register_type::<OwnerPredicted>()
            .register_type::<NetworkOwner>()
            .register_type::<Predicted>()
            .replicate::<Interpolated>()
            .replicate::<NetworkOwner>()
            .replicate::<OwnerPredicted>()
            .configure_sets(PreUpdate, InterpolationSet::Init.after(ClientSet::Receive))
            .configure_sets(
                PreUpdate,
                InterpolationSet::Interpolate.after(InterpolationSet::Init),
            )
            .add_systems(
                Update,
                owner_prediction_init_system
                    .run_if(resource_exists::<NetcodeClientTransport>)
                    .in_set(InterpolationSet::Init),
            )
            .insert_resource(InterpolationConfig {
                max_tick_rate: self.max_tick_rate,
            });
    }
}

#[derive(Resource, Serialize, Deserialize, Debug)]
pub struct InterpolationConfig {
    pub max_tick_rate: u16,
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct Interpolated;

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct OwnerPredicted;

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct NetworkOwner(pub u64);

#[derive(Component, Reflect)]
pub struct Predicted;

pub trait Interpolate {
    fn interpolate(&self, other: Self, t: f32) -> Self;
}

#[derive(Deserialize, Serialize, Reflect)]
pub struct Snapshot<T: Component + Interpolate + Clone> {
    tick: u32,
    value: T,
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct SnapshotBuffer<T: Component + Interpolate + Clone> {
    buffer: VecDeque<T>,
    time_since_last_snapshot: f32,
    latest_snapshot_tick: u32,
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

pub struct EventSnapshot<T: Event> {
    pub value: T,
    pub tick: u32,
    pub delta_time: f32,
}

#[derive(Resource)]
pub struct PredictedEventHistory<T: Event>(pub VecDeque<EventSnapshot<T>>);

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

fn owner_prediction_init_system(
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

/// Initialize snapshot buffers for new entities.
fn snapshot_buffer_init_system<T: Component + Interpolate + Clone>(
    q_new: Query<(Entity, &T), Or<(Added<Predicted>, Added<Interpolated>)>>,
    mut commands: Commands,
    server_ticks: Res<ServerEntityTicks>,
) {
    for (e, initial_value) in q_new.iter() {
        if let Some(tick) = (*server_ticks).get(&e) {
            let mut buffer = SnapshotBuffer::new();
            buffer.insert(initial_value.clone(), tick.get());
            commands.entity(e).insert(buffer);
        }
    }
}

/// Interpolate between snapshots.
fn snapshot_interpolation_system<T: Component + Interpolate + Clone>(
    mut q: Query<(&mut T, &mut SnapshotBuffer<T>), (With<Interpolated>, Without<Predicted>)>,
    time: Res<Time>,
    config: Res<InterpolationConfig>,
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

/// Advances the snapshot buffer time for predicted entities.
fn predicted_snapshot_system<T: Component + Interpolate + Clone>(
    mut q: Query<&mut SnapshotBuffer<T>, (Without<Interpolated>, With<Predicted>)>,
    time: Res<Time>,
) {
    for mut snapshot_buffer in q.iter_mut() {
        snapshot_buffer.time_since_last_snapshot += time.delta_seconds();
    }
}

pub fn deserialize_snap_component<C: Clone + Interpolate + Component + DeserializeOwned>(
    entity: &mut EntityWorldMut,
    _entity_map: &mut ServerEntityMap,
    cursor: &mut Cursor<&[u8]>,
    tick: RepliconTick,
) -> bincode::Result<()> {
    let component: C = deserialize_from(cursor)?;
    if let Some(mut buffer) = entity.get_mut::<SnapshotBuffer<C>>() {
        buffer.insert(component, tick.get());
    } else {
        entity.insert(component);
    }

    Ok(())
}

pub trait AppInterpolationExt {
    /// TODO: Add docs
    fn replicate_interpolated<C>(&mut self) -> &mut Self
    where
        C: Component + Interpolate + Clone + Serialize + DeserializeOwned;

    /// TODO: Add docs
    fn replicate_interpolated_with<C>(
        &mut self,
        serialize: SerializeFn,
        deserialize: DeserializeFn,
        remove: RemoveComponentFn,
    ) -> &mut Self
    where
        C: Component + Interpolate + Clone;

    fn add_client_predicted_event<C>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        C: Event + Serialize + DeserializeOwned + Debug + Clone;
}

impl AppInterpolationExt for App {
    fn replicate_interpolated<C>(&mut self) -> &mut Self
    where
        C: Component + Interpolate + Clone + Serialize + DeserializeOwned,
    {
        self.replicate_interpolated_with::<C>(
            serialize_component::<C>,
            deserialize_snap_component::<C>,
            replication_rules::remove_component::<C>,
        )
    }

    fn replicate_interpolated_with<T>(
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
            (snapshot_buffer_init_system::<T>.after(owner_prediction_init_system))
                .in_set(InterpolationSet::Init)
                .run_if(resource_exists::<RenetClient>),
        )
        .add_systems(
            PreUpdate,
            (
                snapshot_interpolation_system::<T>,
                predicted_snapshot_system::<T>,
            )
                .chain()
                .in_set(InterpolationSet::Interpolate)
                .run_if(resource_exists::<RenetClient>),
        );
        self.replicate_with::<T>(serialize, deserialize, remove)
    }

    fn add_client_predicted_event<C>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        C: Event + Serialize + DeserializeOwned + Debug + Clone,
    {
        let history: PredictedEventHistory<C> = PredictedEventHistory::new();
        self.insert_resource(history);
        self.add_client_event::<C>(channel)
    }
}
