//! Everything that knows Twitch's formats, with no I/O.
//!
//! The service and the tools call into this crate; nothing here opens a socket or a file. The facts
//! about Twitch that go stale (what marks an advert, the client id, the token query) live here so a
//! change by Twitch is one commit in one crate, covered by the fixtures under `fixtures/`.
//!
//! - [`playlist`]: master and media playlists, advert marks, the viewer facts Twitch states.
//! - [`timeline`]: the playlist handed to players, stream segments only, numbered by us.
//! - [`attrs`]: the attribute-list and date parsing the two above share.
//! - [`chat`]: IRC lines, the events a reader sees, the room's rules, the numbered log.
//! - [`emotes`]: the three emote providers' lists and how they merge.
//! - [`gql`]: the GraphQL requests the service sends and the reading of their answers.

pub mod attrs;
pub mod chat;
pub mod emotes;
pub mod gql;
pub mod playlist;
pub mod timeline;

pub use playlist::{
    AD_URLS, AdBreak, CLIENT_ID, KNOWN_CLASSES, MasterInfo, Media, PLAYER_TYPES, STREAM_TITLES,
    Segment, TOKEN_QUERY, Variant, has_ads, master_info, parse_media, variants,
};
pub use timeline::Timeline;
