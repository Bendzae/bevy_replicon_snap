use std::fmt::Debug;
use std::io::Cursor;

use bevy::prelude::*;
use bevy_replicon::core::replication_fns::{
    ctx::{RemoveCtx, WriteCtx},
    rule_fns::RuleFns,
};
use bevy_replicon::core::replicon_channels::RepliconChannel;
use bevy_replicon::prelude::*;
use bevy_replicon::{bincode, core::command_markers::MarkerConfig};
use bevy_replicon_renet::renet::{transport::NetcodeClientTransport, RenetClient};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub use bevy_replicon_snap_macros;

use crate::{
    interpolation::{
        snapshot_interpolation_system, Interpolate, Interpolated, SnapshotBuffer,
        SnapshotInterpolationConfig,
    },
    prediction::{
        owner_prediction_init_system, predicted_snapshot_system, predicted_update_system,
        server_update_system, ApplyEvent, OwnerPredicted, Predicted, PredictedEventHistory,
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

#[derive(Component)]
pub struct RecordSnapshotsMarker;

/// Add a marker to all components requiring a snapshot buffer
pub fn snapshot_buffer_init_system<T: Component + Interpolate + Clone>(
    q_new: Query<(Entity, &T), Or<(Added<Predicted>, Added<Interpolated>)>>,
    mut commands: Commands,
) {
    for (e, _v) in q_new.iter() {
        commands.entity(e).insert(RecordSnapshotsMarker);
    }
}

pub fn write_snap_component<C: Clone + Interpolate + Component + DeserializeOwned>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    let component: C = rule_fns.deserialize(ctx, cursor)?;
    if let Some(mut buffer) = entity.get_mut::<SnapshotBuffer<C>>() {
        buffer.insert(component, ctx.message_tick.get());
    } else {
        let mut buffer = SnapshotBuffer::new();
        buffer.insert(component, ctx.message_tick.get());
        ctx.commands.entity(entity.id()).insert(buffer);
    }

    Ok(())
}

fn remove_snap_component<C: Clone + Interpolate + Component + DeserializeOwned>(
    ctx: &mut RemoveCtx,
    entity: &mut EntityMut,
) {
    ctx.commands
        .entity(entity.id())
        .remove::<SnapshotBuffer<C>>()
        .remove::<C>();
}

pub trait AppInterpolationExt {
    /// TODO: Add docs
    fn replicate_interpolated<C>(&mut self) -> &mut Self
    where
        C: Component + Interpolate + Clone + Serialize + DeserializeOwned;

    /// TODO: Add docs
    fn add_client_predicted_event<E>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone;

    /// TODO: Add docs
    fn predict_event_for_component<E, C>(&mut self) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
        C: Component + ApplyEvent<E> + Clone;
}

impl AppInterpolationExt for App {
    fn replicate_interpolated<T>(&mut self) -> &mut Self
    where
        T: Component + Interpolate + Clone + Serialize + DeserializeOwned,
    {
        self.add_systems(
            PreUpdate,
            (snapshot_buffer_init_system::<T>.after(owner_prediction_init_system))
                .in_set(InterpolationSet::Init)
                .run_if(resource_exists::<RenetClient>),
        );
        self.add_systems(
            PreUpdate,
            (
                snapshot_interpolation_system::<T>,
                predicted_snapshot_system::<T>,
            )
                .chain()
                .in_set(InterpolationSet::Interpolate)
                .run_if(resource_exists::<RenetClient>),
        )
        .replicate::<T>()
        .register_marker_with::<RecordSnapshotsMarker>(MarkerConfig {
            need_history: true,
            ..default()
        })
        .set_marker_fns::<RecordSnapshotsMarker, T>(
            write_snap_component::<T>,
            remove_snap_component::<T>,
        )
    }

    fn add_client_predicted_event<E>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
    {
        let history: PredictedEventHistory<E> = PredictedEventHistory::new();
        self.insert_resource(history);
        self.add_client_event::<E>(channel)
    }

    fn predict_event_for_component<E, C>(&mut self) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
        C: Component + ApplyEvent<E> + Clone,
    {
        self.add_systems(
            Update,
            (
                server_update_system::<E, C>.run_if(has_authority), // Runs only on the server or a single player.
                predicted_update_system::<E, C>.run_if(resource_exists::<RenetClient>), // Runs only on clients.
            ),
        )
    }
}
