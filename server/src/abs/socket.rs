//! The port of Audiobookshelf's socket (`server/SocketAuthority.js`): socket.io on `/socket.io`, the connected
//! clients and the emitters every module uses to tell the apps what changed.
//!
//! A client connects, then sends `auth` with its token; until then it gets no events. The emitters mirror the
//! original's: [`Sockets::emitter`] to every signed-in client (optionally filtered), [`Sockets::client_emitter`]
//! to one user's clients, [`Sockets::admin_emitter`] to admins, and [`Sockets::library_item_emitter`] to the
//! clients allowed to see an item.

use crate::abs::models::{self, user, User};
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use socketioxide::extract::{Data, SocketRef, State};
use socketioxide::layer::SocketIoLayer;
use socketioxide::SocketIo;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One connected client: its socket, and its user once `auth` succeeded.
#[derive(Clone)]
pub struct Client {
    /// The socket, to emit to.
    pub socket: SocketRef,
    /// The signed-in user, `None` before `auth`.
    pub user: Option<User>,
}

/// The connected clients by socket id (the original's `SocketAuthority.clients`). Clones share the same list.
#[derive(Clone, Default)]
pub struct Sockets {
    clients: Arc<Mutex<HashMap<String, Client>>>,
}

/// Whether a user is an admin or the root user (`isAdminOrUp`).
pub fn is_admin_or_up(u: &User) -> bool {
    user::is_admin_or_up(u)
}

impl Sockets {
    /// A snapshot of the clients, so no lock is held while emitting.
    pub fn clients(&self) -> Vec<Client> {
        self.clients.lock().map(|c| c.values().cloned().collect()).unwrap_or_default()
    }

    /// The clients signed in as this user.
    pub fn clients_for_user(&self, user_id: &str) -> Vec<Client> {
        self.clients().into_iter().filter(|c| c.user.as_ref().and_then(|u| u.id.as_deref()) == Some(user_id)).collect()
    }

    /// The signed-in users with how many sockets each has open (`getUsersOnline`): each user's public JSON
    /// plus `connections`.
    pub fn users_online(&self) -> Vec<Value> {
        let mut order: Vec<String> = Vec::new();
        let mut seen: HashMap<String, Value> = HashMap::new();
        for u in self.clients().into_iter().filter_map(|c| c.user) {
            let id = u.id.clone().unwrap_or_default();
            match seen.get_mut(&id) {
                Some(j) => j["connections"] = Value::from(j["connections"].as_i64().unwrap_or(0) + 1),
                None => {
                    let mut j = user::to_json_for_public(&u, None);
                    j["connections"] = Value::from(1);
                    order.push(id.clone());
                    seen.insert(id, j);
                }
            }
        }
        order.into_iter().filter_map(|id| seen.remove(&id)).collect()
    }

    /// Records the user a socket signed in as.
    pub fn set_user(&self, socket_id: &str, user: User) {
        if let Ok(mut c) = self.clients.lock() {
            if let Some(client) = c.get_mut(socket_id) {
                client.user = Some(user);
            }
        }
    }

    /// Sends `event` to every signed-in client whose user passes `filter` (all of them when `None`).
    pub fn emitter(&self, event: &str, data: &impl Serialize, filter: Option<&dyn Fn(&User) -> bool>) {
        for c in self.clients() {
            if let Some(user) = &c.user {
                if filter.is_none_or(|f| f(user)) {
                    let _ = c.socket.emit(event, data);
                }
            }
        }
    }

    /// Sends `event` to every client of one user.
    pub fn client_emitter(&self, user_id: &str, event: &str, data: &impl Serialize) {
        for c in self.clients_for_user(user_id) {
            let _ = c.socket.emit(event, data);
        }
    }

    /// Sends `event` to every admin's (and root's) clients.
    pub fn admin_emitter(&self, event: &str, data: &impl Serialize) {
        self.emitter(event, data, Some(&is_admin_or_up));
    }

    /// Sends `event` with an item's expanded JSON to every client whose user `can_access` it.
    pub fn library_item_emitter(&self, event: &str, item_json: &Value, can_access: &dyn Fn(&User) -> bool) {
        self.emitter(event, item_json, Some(can_access));
    }

    /// Sends `event` with the items each client's user `can_access` (the original's `libraryItemsEmitter`):
    /// `items` are (item, expanded JSON) pairs; a client that may see none of them gets nothing.
    pub fn library_items_emitter<T>(&self, event: &str, items: &[(T, Value)], can_access: &dyn Fn(&User, &T) -> bool) {
        for c in self.clients() {
            if let Some(user) = &c.user {
                let visible: Vec<&Value> = items.iter().filter(|(i, _)| can_access(user, i)).map(|(_, j)| j).collect();
                if !visible.is_empty() {
                    let _ = c.socket.emit(event, &visible);
                }
            }
        }
    }

    fn add(&self, socket: SocketRef) {
        if let Ok(mut c) = self.clients.lock() {
            c.insert(socket.id.to_string(), Client { socket, user: None });
        }
    }

    fn remove(&self, socket_id: &str) -> Option<Client> {
        self.clients.lock().ok().and_then(|mut c| c.remove(socket_id))
    }
}

/// An event with no arguments (`socket.emit('pong')` in the original): an empty tuple is sent as no data.
#[derive(Serialize)]
pub struct NoArgs();

/// The socket.io layer for the app's router, with the original's events.
pub fn layer(st: AppState) -> SocketIoLayer {
    let (layer, io) = SocketIo::builder().with_state(st).build_layer();
    io.ns("/", on_connect);
    layer
}

/// A new connection: kept as a client, with the original's events.
async fn on_connect(socket: SocketRef, State(st): State<AppState>) {
    st.sockets.add(socket.clone());
    socket.on("auth", on_auth);
    socket.on("ping", async |s: SocketRef| {
        let _ = s.emit("pong", &NoArgs());
    });
    socket.on_disconnect(async |s: SocketRef, State(st): State<AppState>| {
        if let Some(u) = st.sockets.remove(&s.id.to_string()).and_then(|c| c.user) {
            st.sockets.admin_emitter("user_offline", &user::to_json_for_public(&u, None));
        }
    });
}

/// `auth`: signs the socket in with the token the app holds (`authenticateSocket`), or answers `auth_failed`.
/// Admins are told the user came online; the user's `lastSeen` is updated; the socket gets `init` (with who is
/// online, for admins).
async fn on_auth(socket: SocketRef, Data(token): Data<Value>, State(st): State<AppState>) {
    let token = token.as_str().unwrap_or_default().to_string();
    let mut user = match crate::abs::socket_user(&st, &token) {
        Ok(u) => u,
        Err(message) => {
            tracing::warn!("A socket couldn't sign in: {message}");
            let _ = socket.emit("auth_failed", &serde_json::json!({ "message": message }));
            return;
        }
    };
    st.sockets.set_user(&socket.id.to_string(), user.clone());
    st.sockets.admin_emitter("user_online", &user::to_json_for_public(&user, None));
    user.last_seen = Some(models::date_now());
    if let Err(e) = models::update(&st.abs, &user) {
        tracing::warn!("The user's last seen time couldn't be saved: {}", e.detail);
    }
    let mut payload = serde_json::json!({ "userId": user.id, "username": user.username });
    if user::is_admin_or_up(&user) {
        payload["usersOnline"] = Value::Array(st.sockets.users_online());
    }
    let _ = socket.emit("init", &payload);
}
