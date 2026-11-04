//! What goes over the wire: the messages of `docs/multiplayer.typ`, and their codec.
//!
//! Every message is a value of a type here, encoded with `postcard`. A decoder is handed bytes
//! from a client it cannot trust, so [`decode`] is total: it returns an error for anything that
//! is not exactly one message of the type, and never panics or allocates more than the input
//! is long.

use nabla_core::{Params, State};
use nabla_sim::command::Command;
use nabla_sim::world::ShipRequest;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The version of this protocol, which `Hello` carries.
pub const PROTOCOL: u16 = 2;

/// The most a stream frame may hold: a world file is at most 1 MiB, and a `Lobby` holds one.
pub const MAX_STREAM_FRAME: usize = 1024 * 1024 + 4096;

/// The most a client's stream frame may hold: `Hello`, `Join` and `Leave` are far smaller. The
/// server's glue refuses longer frames before it reads them.
pub const MAX_CLIENT_FRAME: usize = 4096;

/// The most commands a `Commands` datagram may carry: `K` is far below it.
pub const MAX_COMMANDS: usize = 32;

/// A connection, which is a pilot's identity (assumption A2): the server never trusts a field of
/// a message for who sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConnId(pub u32);

/// A ship in a session. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipId(pub u32);

/// A tick number: the only time the protocol's correctness rests on.
pub type Tick = u32;

/// The longest a [`Username`] may be.
pub const MAX_USERNAME: usize = 32;

/// The name a pilot flies under: 1 to [`MAX_USERNAME`] lowercase ASCII letters and digits.
///
/// Every value of this type is valid, decoded ones too, so a name that breaks the rule is not a
/// message at all. It says only what a pilot wants to be called: who sent a message is still its
/// connection (assumption A2).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Username(String);

impl Username {
    /// `text` as a name.
    ///
    /// # Errors
    /// If it is empty, too long, or holds anything but lowercase ASCII letters and digits.
    pub fn parse(text: &str) -> Result<Self, InvalidUsername> {
        Self::try_from(text.to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Username {
    type Error = InvalidUsername;

    fn try_from(text: String) -> Result<Self, InvalidUsername> {
        let allowed = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
        if (1..=MAX_USERNAME).contains(&text.len()) && text.bytes().all(allowed) {
            Ok(Self(text))
        } else {
            Err(InvalidUsername)
        }
    }
}

impl std::fmt::Display for Username {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why text is not a [`Username`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidUsername;

impl std::fmt::Display for InvalidUsername {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a username is 1 to {MAX_USERNAME} lowercase letters and digits"
        )
    }
}

/// What a client says on its stream.
///
/// A new variant goes last, and `Hello` stays first and as it is: a client of any version must be
/// able to say `Hello`, and be told if the server does not speak its protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientStream {
    /// The first message of a connection: the protocol version, and the build, for the log.
    Hello { protocol: u16, build: String },
    /// Take a ship, as close to this request as the world allows, under this name.
    Join {
        request: ShipRequest,
        username: Username,
    },
    /// Give the ship up. The connection stays, back in the lobby.
    Leave,
    /// The pilot has left this app for another (`true`), or has come back to it (`false`). It says
    /// only how the ship's name is shown, and nothing about its flight.
    Away(bool),
}

/// What the server says on a stream.
///
/// A new variant goes last, so that a client of another version still understands `Rejected`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerStream {
    Lobby(Lobby),
    Welcome(Welcome),
    /// There is no room: the server holds as many pilots as it allows.
    Full,
    /// The connection cannot go on, and why.
    Rejected(String),
    /// A ship exists for a stretch of ticks. Sent again when its pilot's activity changes.
    Roster(RosterEntry),
    /// A ship in the world is already flying under the name asked for. The client stays in the
    /// lobby.
    NameTaken,
}

/// The world, as a validated world file with its tick rate, arena and allowed ships, and who is
/// flying in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lobby {
    pub world: String,
    pub pilots: Vec<ShipId>,
    pub max_pilots: u8,
}

/// The pilot's ship, as granted, and the world at the tick it joined.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub ship: ShipId,
    pub params: Params,
    /// `T_0`: the server's tick when the ship joined. The ship's log begins here.
    pub tick: Tick,
    pub ships: Vec<ShipRecord>,
}

/// Ship `ship` exists for the ticks in `[since, until)`, with these parameters, flown by the pilot
/// named `username`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub ship: ShipId,
    pub params: Params,
    pub since: Tick,
    pub until: Option<Tick>,
    pub username: Username,
    /// Whether the pilot is at the controls, as far as the server can tell. It is for showing
    /// beside the name, and no rule of the protocol depends on it.
    pub active: bool,
}

/// One ship in a snapshot: `(j, s_j, eta_j, u_j)`. The control is for drawing the plume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShipRecord {
    pub ship: ShipId,
    pub state: State,
    pub hold: Option<f64>,
    pub thrust: f32,
    pub torque: f32,
}

/// What a client sends up, in a datagram.
///
/// The command stamped `stamp`, and the ones before it, `cmds[j]` stamped `stamp - j` (`None`
/// where the client issued none), so that a lost datagram is made up for by the `K - 1` that
/// follow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Commands {
    pub stamp: Tick,
    pub cmds: Vec<Option<Command>>,
}

/// What the server sends down, in a datagram: the world at tick `label`, and how early the
/// client's newest command came (negative: it came late), in microseconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub label: Tick,
    pub margin_us: i32,
    pub ships: Vec<ShipRecord>,
}

/// Why bytes are not a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// More bytes than `limit`.
    TooLong,
    /// Not a value of the type, or a value followed by more bytes.
    Malformed,
}

/// The message as bytes.
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    postcard::to_stdvec(message).expect("a message of these types always serializes")
}

/// Exactly one message of type `T` from `bytes`, no longer than `limit`.
///
/// # Errors
/// [`CodecError::TooLong`] past `limit`, and [`CodecError::Malformed`] if the bytes are not one
/// `T` and nothing else.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, CodecError> {
    if bytes.len() > limit {
        return Err(CodecError::TooLong);
    }
    let (message, rest) =
        postcard::take_from_bytes(bytes).map_err(|_error| CodecError::Malformed)?;
    if rest.is_empty() {
        Ok(message)
    } else {
        Err(CodecError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_sim::rng::Rng;
    use nabla_sim::steering::{Aim, Input, Steering};

    fn record(ship: u32) -> ShipRecord {
        ShipRecord {
            ship: ShipId(ship),
            state: State {
                x: -15.5,
                y: 3.25,
                vx: 4.0,
                vy: -4.0,
                theta: 1.5,
                omega: 2.0,
            },
            hold: Some(0.5),
            thrust: 5.0,
            torque: -1.0,
        }
    }

    #[test]
    fn messages_round_trip() {
        let commands = Commands {
            stamp: 7,
            cmds: vec![
                Some(Command::Manual {
                    input: Input {
                        thrust: 1.0,
                        turn: -0.5,
                        fine: true,
                        aim: Some(Aim {
                            heading: 1.0,
                            cone: 0.2,
                        }),
                    },
                    steering: Steering::Sas,
                }),
                None,
                Some(Command::Direct {
                    thrust: 1.0,
                    torque: 2.0,
                }),
            ],
        };
        assert_eq!(decode::<Commands>(&encode(&commands), 1150), Ok(commands));
        let snapshot = Snapshot {
            label: 9,
            margin_us: -250,
            ships: vec![record(0), record(1)],
        };
        assert_eq!(decode::<Snapshot>(&encode(&snapshot), 1150), Ok(snapshot));
        let hello = ClientStream::Hello {
            protocol: PROTOCOL,
            build: "test".to_owned(),
        };
        assert_eq!(decode::<ClientStream>(&encode(&hello), 100), Ok(hello));
        let join = ClientStream::Join {
            request: ShipRequest {
                mass: 1.0,
                inertia: 2.0,
                f_max: 3.0,
                tau_max: 4.0,
            },
            username: Username::parse("miriam").expect("a valid name"),
        };
        assert_eq!(decode::<ClientStream>(&encode(&join), 100), Ok(join));
        let away = ClientStream::Away(true);
        assert_eq!(decode::<ClientStream>(&encode(&away), 100), Ok(away));
        let taken = ServerStream::NameTaken;
        assert_eq!(decode::<ServerStream>(&encode(&taken), 100), Ok(taken));
    }

    #[test]
    fn a_username_is_one_to_32_lowercase_letters_and_digits() {
        let longest = "abcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(longest.len(), MAX_USERNAME);
        for good in ["a", "7", "bot7", "start2ep", longest] {
            assert_eq!(Username::parse(good).expect("valid").as_str(), good);
        }
        let too_long = format!("{longest}6");
        for bad in [
            "",
            too_long.as_str(),
            "Miriam",
            "two words",
            "under_score",
            "dash-ed",
            "jos\u{e9}",
            "\u{0}",
        ] {
            assert_eq!(Username::parse(bad), Err(InvalidUsername), "{bad:?}");
        }
    }

    #[test]
    fn a_name_that_breaks_the_rule_does_not_decode() {
        // What a client outside this codebase could send: the same bytes as a valid name's.
        for bad in ["", "Miriam", "under_score", "jos\u{e9}"] {
            let bytes = encode(&bad.to_owned());
            assert_eq!(
                decode::<Username>(&bytes, 100),
                Err(CodecError::Malformed),
                "{bad:?}"
            );
        }
        let too_long = encode(&"a".repeat(MAX_USERNAME + 1));
        assert_eq!(
            decode::<Username>(&too_long, 100),
            Err(CodecError::Malformed)
        );
        let good = encode(&"miriam".to_owned());
        assert_eq!(
            decode::<Username>(&good, 100),
            Ok(Username::parse("miriam").expect("a valid name"))
        );
    }

    #[test]
    fn a_client_of_another_version_is_still_understood_and_told() {
        // An older client's Hello, byte for byte: variant 0, protocol 1, a build of one byte.
        assert_eq!(
            decode::<ClientStream>(&[0, 1, 1, b'x'], 100),
            Ok(ClientStream::Hello {
                protocol: 1,
                build: "x".to_owned(),
            })
        );
        // And the answer it can read: variant 3 of what it knows of the server, as always.
        assert_eq!(
            encode(&ServerStream::Rejected("x".to_owned())),
            [3, 1, b'x']
        );
    }

    #[test]
    fn a_snapshot_of_sixteen_ships_fits_one_datagram() {
        // R1: whole snapshots, never fragmented, in the datagram QUIC guarantees (1150 bytes
        // after headers).
        let snapshot = Snapshot {
            label: u32::MAX,
            margin_us: i32::MIN,
            ships: (0..16).map(|i| record(1000 + i)).collect(),
        };
        let bytes = encode(&snapshot);
        assert!(bytes.len() <= 1150, "{} bytes", bytes.len());
    }

    #[test]
    fn decoding_garbage_never_panics_and_rejects_what_is_not_one_message() {
        let mut rng = Rng::new(0xC0DEC);
        let mut accepted = 0;
        for _ in 0..200_000 {
            let len = rng.below(80) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
            accepted += usize::from(decode::<Commands>(&bytes, 1150).is_ok());
            drop(decode::<Snapshot>(&bytes, 1150));
            drop(decode::<ClientStream>(&bytes, 1150));
        }
        assert!(accepted < 200_000, "most garbage is not a message");
        let good = encode(&Commands {
            stamp: 1,
            cmds: vec![None],
        });
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            decode::<Commands>(&trailing, 1150),
            Err(CodecError::Malformed)
        );
        assert_eq!(decode::<Commands>(&good, 2), Err(CodecError::TooLong));
        // A length prefix that promises far more than is there allocates nothing of the kind.
        assert_eq!(
            decode::<Commands>(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F], 1150),
            Err(CodecError::Malformed)
        );
    }
}
