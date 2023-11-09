//! This is the "Simple Box" example from the bevy_replicon repo with owner predicted players
//! This means the local player is predicted and other networked entities are interpolated

use std::{
    error::Error,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::prelude::*;
use bevy_replicon::renet::ClientId;
use bevy_replicon::{
    prelude::*,
    renet::{
        transport::{
            ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport,
            ServerAuthentication, ServerConfig,
        },
        ConnectionConfig, ServerEvent,
    },
};
use bevy_replicon_snap::{
    AppInterpolationExt, Interpolated, NetworkOwner, OwnerPredicted, Predicted,
    PredictedEventHistory, SnapshotBuffer, SnapshotInterpolationPlugin,
};
use bevy_replicon_snap_macros::{Interpolate, SnapDeserialize, SnapSerialize};
use clap::Parser;
use serde::{Deserialize, Serialize};

// Setting a overly low server tickrate to make the difference between the different methods clearly visible
// Usually you would want a server for a realtime game to run with at least 30 ticks per second
const MAX_TICK_RATE: u16 = 5;

fn main() {
    App::new()
        .init_resource::<Cli>() // Parse CLI before creating window.
        .add_plugins((
            DefaultPlugins,
            ReplicationPlugins
                .build()
                .set(ServerPlugin::new(TickPolicy::MaxTickRate(MAX_TICK_RATE))),
            SnapshotInterpolationPlugin {
                max_tick_rate: MAX_TICK_RATE,
            },
            SimpleBoxPlugin,
        ))
        .run();
}

struct SimpleBoxPlugin;

impl Plugin for SimpleBoxPlugin {
    fn build(&self, app: &mut App) {
        app.replicate_interpolated::<PlayerPosition>()
            .replicate::<PlayerColor>()
            .add_client_predicted_event::<MoveDirection>(EventType::Ordered)
            .add_systems(
                Startup,
                (
                    Self::cli_system.pipe(system_adapter::unwrap),
                    Self::init_system,
                ),
            )
            .add_systems(
                Update,
                (
                    Self::movement_system.run_if(has_authority()), // Runs only on the server or a single player.
                    Self::predicted_movement_system.run_if(resource_exists::<RenetClient>()), // Runs only on clients.
                    Self::server_event_system.run_if(resource_exists::<RenetServer>()), // Runs only on the server.
                    (Self::draw_boxes_system, Self::input_system),
                ),
            );
    }
}

impl SimpleBoxPlugin {
    fn cli_system(
        mut commands: Commands,
        cli: Res<Cli>,
        network_channels: Res<NetworkChannels>,
    ) -> Result<(), Box<dyn Error>> {
        match *cli {
            Cli::SinglePlayer => {
                commands.spawn(PlayerBundle::new(SERVER_ID, Vec2::ZERO, Color::GREEN));
            }
            Cli::Server { port } => {
                let server_channels_config = network_channels.get_server_configs();
                let client_channels_config = network_channels.get_client_configs();

                let server = RenetServer::new(ConnectionConfig {
                    server_channels_config,
                    client_channels_config,
                    ..Default::default()
                });

                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let public_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
                let socket = UdpSocket::bind(public_addr)?;
                let server_config = ServerConfig {
                    current_time,
                    max_clients: 10,
                    protocol_id: PROTOCOL_ID,
                    authentication: ServerAuthentication::Unsecure,
                    public_addresses: vec![public_addr],
                };
                let transport = NetcodeServerTransport::new(server_config, socket)?;

                commands.insert_resource(server);
                commands.insert_resource(transport);

                commands.spawn(TextBundle::from_section(
                    "Server",
                    TextStyle {
                        font_size: 30.0,
                        color: Color::WHITE,
                        ..default()
                    },
                ));
                commands.spawn(PlayerBundle::new(SERVER_ID, Vec2::ZERO, Color::GREEN));
            }
            Cli::Client { port, ip } => {
                let server_channels_config = network_channels.get_server_configs();
                let client_channels_config = network_channels.get_client_configs();

                let client = RenetClient::new(ConnectionConfig {
                    server_channels_config,
                    client_channels_config,
                    ..Default::default()
                });

                let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
                let client_id = current_time.as_millis() as u64;
                let server_addr = SocketAddr::new(ip, port);
                let socket = UdpSocket::bind((ip, 0))?;
                let authentication = ClientAuthentication::Unsecure {
                    client_id,
                    protocol_id: PROTOCOL_ID,
                    server_addr,
                    user_data: None,
                };
                let transport = NetcodeClientTransport::new(current_time, authentication, socket)?;

                commands.insert_resource(client);
                commands.insert_resource(transport);

                commands.spawn(TextBundle::from_section(
                    format!("Client: {client_id:?}"),
                    TextStyle {
                        font_size: 30.0,
                        color: Color::WHITE,
                        ..default()
                    },
                ));
            }
        }

        Ok(())
    }

    fn init_system(mut commands: Commands) {
        commands.spawn(Camera2dBundle::default());
    }

    /// Logs server events and spawns a new player whenever a client connects.
    fn server_event_system(mut commands: Commands, mut server_event: EventReader<ServerEvent>) {
        for event in server_event.read() {
            match event {
                ServerEvent::ClientConnected { client_id } => {
                    info!("player: {client_id} Connected");
                    // Generate pseudo random color from client id.
                    let r = ((client_id.raw() % 23) as f32) / 23.0;
                    let g = ((client_id.raw() % 27) as f32) / 27.0;
                    let b = ((client_id.raw() % 39) as f32) / 39.0;
                    commands.spawn(PlayerBundle::new(
                        *client_id,
                        Vec2::ZERO,
                        Color::rgb(r, g, b),
                    ));
                }
                ServerEvent::ClientDisconnected { client_id, reason } => {
                    info!("client {client_id} disconnected: {reason}");
                }
            }
        }
    }

    fn draw_boxes_system(mut gizmos: Gizmos, players: Query<(&PlayerPosition, &PlayerColor)>) {
        for (position, color) in &players {
            gizmos.rect(
                Vec3::new(position.x, position.y, 0.0),
                Quat::IDENTITY,
                Vec2::ONE * 50.0,
                color.0,
            );
        }
    }

    /// Reads player inputs and sends [`MoveCommandEvents`]
    fn input_system(mut move_events: EventWriter<MoveDirection>, input: Res<Input<KeyCode>>) {
        let mut direction = Vec2::ZERO;
        if input.pressed(KeyCode::Right) {
            direction.x += 1.0;
        }
        if input.pressed(KeyCode::Left) {
            direction.x -= 1.0;
        }
        if input.pressed(KeyCode::Up) {
            direction.y += 1.0;
        }
        if input.pressed(KeyCode::Down) {
            direction.y -= 1.0;
        }
        if direction != Vec2::ZERO {
            move_events.send(MoveDirection(direction.normalize_or_zero()));
        }
    }

    /// Mutates [`PlayerPosition`] based on [`MoveCommandEvents`].
    /// Server implementation
    fn movement_system(
        time: Res<Time>,
        mut move_events: EventReader<FromClient<MoveDirection>>,
        mut players: Query<(&NetworkOwner, &mut PlayerPosition), Without<Predicted>>,
    ) {
        for FromClient { client_id, event } in move_events.read() {
            info!("received event {event:?} from client {client_id}");
            for (player, mut position) in &mut players {
                if client_id.raw() == player.0 {
                    Self::apply_move_command(&mut *position, event, time.delta_seconds())
                }
            }
        }
    }

    // Client prediction implementation
    fn predicted_movement_system(
        mut q_predicted_players: Query<
            (&mut PlayerPosition, &SnapshotBuffer<PlayerPosition>),
            (With<Predicted>, Without<Interpolated>),
        >,
        mut local_events: EventReader<MoveDirection>,
        mut event_history: ResMut<PredictedEventHistory<MoveDirection>>,
        client_tick: Res<LastRepliconTick>,
        time: Res<Time>,
    ) {
        // Append the latest input event
        for event in local_events.read() {
            event_history.insert(event.clone(), client_tick.get(), time.delta_seconds());
        }
        // Apply all pending inputs to latest snapshot
        for (mut position, snapshot_buffer) in q_predicted_players.iter_mut() {
            let mut corrected_position = snapshot_buffer.latest_snapshot().0;
            for event_snapshot in event_history.predict(snapshot_buffer.latest_snapshot_tick()) {
                Self::apply_move_command(
                    &mut corrected_position,
                    &event_snapshot.value,
                    event_snapshot.delta_time,
                );
            }
            position.0 = corrected_position;
        }
    }

    fn apply_move_command(position: &mut Vec2, event: &MoveDirection, delta_time: f32) {
        const MOVE_SPEED: f32 = 300.0;
        *position += event.0 * delta_time * MOVE_SPEED;
    }
}

const PORT: u16 = 5000;
const PROTOCOL_ID: u64 = 0;

#[derive(Debug, Parser, PartialEq, Resource)]
enum Cli {
    SinglePlayer,
    Server {
        #[arg(short, long, default_value_t = PORT)]
        port: u16,
    },
    Client {
        #[arg(short, long, default_value_t = Ipv4Addr::LOCALHOST.into())]
        ip: IpAddr,

        #[arg(short, long, default_value_t = PORT)]
        port: u16,
    },
}

impl Default for Cli {
    fn default() -> Self {
        Self::parse()
    }
}

#[derive(Bundle)]
struct PlayerBundle {
    owner: NetworkOwner,
    position: PlayerPosition,
    color: PlayerColor,
    replication: Replication,
    owner_predicted: OwnerPredicted,
}

impl PlayerBundle {
    fn new(id: ClientId, position: Vec2, color: Color) -> Self {
        Self {
            owner: NetworkOwner(id.raw()),
            position: PlayerPosition(position),
            color: PlayerColor(color),
            replication: Replication,
            owner_predicted: OwnerPredicted,
        }
    }
}

#[derive(
    Component,
    Deserialize,
    Serialize,
    Deref,
    DerefMut,
    Interpolate,
    SnapSerialize,
    SnapDeserialize,
    Clone,
)]
struct PlayerPosition(Vec2);

#[derive(Component, Deserialize, Serialize)]
struct PlayerColor(Color);

/// A movement event for the controlled box.
#[derive(Debug, Default, Deserialize, Event, Serialize, Clone)]
struct MoveDirection(Vec2);
