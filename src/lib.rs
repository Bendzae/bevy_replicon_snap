use std::fmt::Debug;
use std::io::Cursor;

use bevy::prelude::*;
use bevy_replicon::bincode;
use bevy_replicon::bincode::deserialize_from;
use bevy_replicon::client::client_mapper::ServerEntityMap;
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

use crate::{
    interpolation::{
        snapshot_buffer_init_system, snapshot_interpolation_system, Interpolate, Interpolated,
        SnapshotBuffer, SnapshotInterpolationConfig,
    },
    prediction::{
        owner_prediction_init_system, predicted_snapshot_system, OwnerPredicted, Predicted,
        PredictedEventHistory,
    },
};

pub mod interpolation;
pub mod prediction;

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
            .insert_resource(SnapshotInterpolationConfig {
                max_tick_rate: self.max_tick_rate,
            });
    }
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct NetworkOwner(pub u64);

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
