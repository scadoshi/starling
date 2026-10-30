//! Microsoft Graph calls: the signed-in user, chats, their members and messages.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::message;

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Group chats with no topic are named after this many members.
const NAMED_MEMBERS: usize = 3;

/// One connection pool for every Graph and login request.
pub(crate) static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ChatMember {
    /// The membership's ID, which differs from the user's.
    pub(crate) id: Option<String>,
    /// The member's Entra user ID, the `id` that `/me` returns. Absent for members from outside the directory.
    #[serde(rename = "userId")]
    pub(crate) user_id: Option<String>,
    #[serde(rename = "displayName")]
    pub(crate) display_name: Option<String>,
    pub(crate) email: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Chat {
    pub(crate) id: String,
    pub(crate) topic: Option<String>,
    /// `oneOnOne`, `group` or `meeting`.
    #[serde(rename = "chatType")]
    pub(crate) kind: String,
    #[serde(rename = "lastUpdatedDateTime")]
    pub(crate) last_updated: Option<String>,
    /// Everyone in the chat except the signed-in user.
    #[serde(skip)]
    pub(crate) members: Vec<ChatMember>,
    /// The name shown in the chat list, without control characters.
    #[serde(skip)]
    pub(crate) display_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Message {
    pub(crate) id: String,
    /// RFC 3339, UTC.
    #[serde(rename = "createdDateTime")]
    pub(crate) created_date_time: String,
    pub(crate) from: Option<MessageFrom>,
    pub(crate) body: Option<MessageBody>,
    #[serde(default)]
    pub(crate) attachments: Option<Vec<MessageAttachment>>,
}

impl Message {
    /// The sender's display name, or "Unknown" for system messages and apps.
    pub(crate) fn sender_name(&self) -> &str {
        self.from
            .as_ref()
            .and_then(|f| f.user.as_ref())
            .and_then(|u| u.display_name.as_deref())
            .unwrap_or("Unknown")
    }

    /// The body as Graph sent it, HTML for most messages.
    pub(crate) fn content(&self) -> &str {
        self.body
            .as_ref()
            .and_then(|b| b.content.as_deref())
            .unwrap_or_default()
    }

    pub(crate) fn attachment(&self, id: &str) -> Option<&MessageAttachment> {
        self.attachments.as_ref()?.iter().find(|a| a.id == id)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct MessageAttachment {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    #[serde(rename = "contentType")]
    pub(crate) content_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct MessageFrom {
    pub(crate) user: Option<MessageUser>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct MessageUser {
    #[serde(rename = "displayName")]
    pub(crate) display_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct MessageBody {
    pub(crate) content: Option<String>,
}

/// Graph's wrapper around a list.
#[derive(Debug, Deserialize)]
struct Page<T> {
    value: Vec<T>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct User {
    #[serde(rename = "displayName")]
    pub(crate) display_name: String,
    pub(crate) id: String,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    body: SendMessageBody<'a>,
}

#[derive(Debug, Serialize)]
struct SendMessageBody<'a> {
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct MarkChatReadRequest {
    user: MarkChatReadUser,
}

#[derive(Debug, Serialize)]
struct MarkChatReadUser {
    id: String,
    #[serde(rename = "tenantId")]
    tenant_id: String,
}

#[derive(Debug, Deserialize)]
struct Organization {
    id: String,
}

async fn fetch_json<T: DeserializeOwned>(access_token: &str, path: &str) -> Result<T> {
    let response = CLIENT
        .get(format!("{GRAPH_API_BASE}{path}"))
        .bearer_auth(access_token)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("GET {path} failed: {status} - {text}");
    }
    Ok(response.json().await?)
}

fn profile_path() -> Result<PathBuf> {
    Ok(crate::config::app_dir()?.join("profile.json"))
}

fn load_profile() -> Result<Option<User>> {
    let path = profile_path()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&fs::read_to_string(path)?)?))
}

fn save_profile(user: &User) -> Result<()> {
    crate::config::write_private(&profile_path()?, &serde_json::to_string_pretty(user)?)
}

/// The signed-in user, from `profile.json` when it is cached there.
pub(crate) async fn fetch_me(access_token: &str) -> Result<User> {
    if let Ok(Some(user)) = load_profile() {
        return Ok(user);
    }

    let user: User = fetch_json(access_token, "/me")
        .await
        .context("Failed to get user profile")?;
    // The cache only saves a request next time, so failing to write it is not an error.
    save_profile(&user).ok();
    Ok(user)
}

/// The chat's messages, newest first.
pub(crate) async fn fetch_messages(access_token: &str, chat_id: &str) -> Result<Vec<Message>> {
    let page: Page<Message> =
        fetch_json(access_token, &format!("/chats/{chat_id}/messages")).await?;
    Ok(page.value)
}

/// Everything the chat screen needs to open.
#[derive(Debug)]
pub(crate) struct Startup {
    /// `None` when the profile could not be fetched.
    pub(crate) user_name: Option<String>,
    pub(crate) chats: Vec<Chat>,
    /// Chat ID to the time of its newest message. Chats that failed or are empty are left out.
    pub(crate) last_message_times: HashMap<String, String>,
}

/// The user's name, their chats, and the newest message time of every chat, fetched at once.
pub(crate) async fn fetch_startup(access_token: &str) -> Result<Startup> {
    let user_name = fetch_me(access_token)
        .await
        .ok()
        .map(|user| user.display_name);
    let chats = fetch_chats(access_token)
        .await
        .context("Failed to fetch chats")?;

    let tasks: Vec<_> = chats
        .iter()
        .map(|chat| {
            let token = access_token.to_owned();
            let chat_id = chat.id.clone();
            tokio::spawn(async move {
                let messages = fetch_messages(&token, &chat_id).await.ok()?;
                let time = crate::app::newest(&messages)?.created_date_time.clone();
                Some((chat_id, time))
            })
        })
        .collect();

    let mut last_message_times = HashMap::new();
    for task in tasks {
        if let Ok(Some((chat_id, time))) = task.await {
            last_message_times.insert(chat_id, time);
        }
    }
    Ok(Startup {
        user_name,
        chats,
        last_message_times,
    })
}

/// Posts `content` to the chat as plain text.
pub(crate) async fn send_message(access_token: &str, chat_id: &str, content: &str) -> Result<()> {
    let response = CLIENT
        .post(format!("{GRAPH_API_BASE}/chats/{chat_id}/messages"))
        .bearer_auth(access_token)
        .json(&SendMessageRequest {
            body: SendMessageBody { content },
        })
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        anyhow::bail!("Failed to send message: {status} - {text}");
    }
    Ok(())
}

/// Marks the chat read for the signed-in user. Best effort: any failure is ignored.
pub(crate) async fn mark_chat_as_read(access_token: &str, chat_id: &str) {
    let Ok(user) = fetch_me(access_token).await else {
        return;
    };
    let Ok(organizations) = fetch_json::<Page<Organization>>(access_token, "/organization").await
    else {
        return;
    };
    let Some(tenant_id) = organizations.value.into_iter().next().map(|org| org.id) else {
        return;
    };

    // A failure here leaves the chat unread in other clients, and nothing on screen depends on it.
    CLIENT
        .post(format!(
            "{GRAPH_API_BASE}/chats/{chat_id}/markChatReadForUser"
        ))
        .bearer_auth(access_token)
        .json(&MarkChatReadRequest {
            user: MarkChatReadUser {
                id: user.id,
                tenant_id,
            },
        })
        .send()
        .await
        .ok();
}

/// Every chat of the signed-in user, with members and a display name filled in.
///
/// The signed-in user is left out of each member list when their profile is available.
pub(crate) async fn fetch_chats(access_token: &str) -> Result<Vec<Chat>> {
    let my_id = fetch_me(access_token).await.ok().map(|user| user.id);
    let mut chats = fetch_json::<Page<Chat>>(access_token, "/me/chats")
        .await?
        .value;

    for chat in &mut chats {
        let members =
            fetch_json::<Page<ChatMember>>(access_token, &format!("/chats/{}/members", chat.id))
                .await
                .map(|page| page.value)
                .unwrap_or_default();
        chat.members = others(members, my_id.as_deref());
        chat.display_name = message::sanitize(&display_name(chat));
    }
    Ok(chats)
}

/// `members` without the user with ID `my_id`.
fn others(mut members: Vec<ChatMember>, my_id: Option<&str>) -> Vec<ChatMember> {
    if let Some(my_id) = my_id {
        members.retain(|m| m.user_id.as_deref() != Some(my_id));
    }
    members
}

/// The name for the chat list: the other person in a one-on-one, else the topic, else up to three members, else a placeholder.
fn display_name(chat: &Chat) -> String {
    if chat.kind == "oneOnOne" {
        return chat
            .members
            .first()
            .and_then(|m| m.display_name.clone())
            .unwrap_or_else(|| "Unknown".to_owned());
    }

    if let Some(topic) = chat.topic.as_deref().filter(|t| !t.is_empty()) {
        return topic.to_owned();
    }

    let names: Vec<String> = chat
        .members
        .iter()
        .filter_map(|m| m.display_name.as_deref().map(abbreviate_name))
        .take(NAMED_MEMBERS)
        .collect();
    if !names.is_empty() {
        return names.join(", ");
    }

    match chat.kind.as_str() {
        "meeting" => "Unnamed Meeting",
        "group" => "Unnamed Group",
        _ => "Unknown Chat",
    }
    .to_owned()
}

/// "Ada Byron Lovelace" becomes "Ada L".
fn abbreviate_name(full_name: &str) -> String {
    let mut parts = full_name.split_whitespace();
    let Some(first) = parts.next() else {
        return String::new();
    };
    match parts.next_back().and_then(|last| last.chars().next()) {
        Some(initial) => format!("{first} {initial}"),
        None => first.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::chat;

    fn member(name: &str) -> ChatMember {
        ChatMember {
            id: None,
            user_id: None,
            display_name: Some(name.to_owned()),
            email: None,
        }
    }

    fn chat_of(kind: &str, topic: Option<&str>, members: &[&str]) -> Chat {
        Chat {
            kind: kind.to_owned(),
            topic: topic.map(str::to_owned),
            members: members.iter().map(|m| member(m)).collect(),
            ..chat("id")
        }
    }

    #[test]
    fn abbreviate_name_keeps_first_name_and_last_initial() {
        assert_eq!(abbreviate_name("Ada Byron Lovelace"), "Ada L");
        assert_eq!(abbreviate_name("Ada"), "Ada");
        assert_eq!(abbreviate_name("  "), "");
        assert_eq!(abbreviate_name("Émile Ångström"), "Émile Å");
    }

    #[test]
    fn display_name_for_one_on_one_is_the_other_person() {
        assert_eq!(
            display_name(&chat_of("oneOnOne", Some("ignored"), &["Grace Hopper"])),
            "Grace Hopper"
        );
        assert_eq!(display_name(&chat_of("oneOnOne", None, &[])), "Unknown");
    }

    #[test]
    fn display_name_prefers_a_topic_then_members_then_a_placeholder() {
        assert_eq!(
            display_name(&chat_of("group", Some("Launch"), &["A B"])),
            "Launch"
        );
        assert_eq!(
            display_name(&chat_of(
                "group",
                Some(""),
                &["Ann Lee", "Bo Ma", "Cy Ng", "Di Oh"]
            )),
            "Ann L, Bo M, Cy N"
        );
        assert_eq!(
            display_name(&chat_of("meeting", None, &[])),
            "Unnamed Meeting"
        );
        assert_eq!(display_name(&chat_of("group", None, &[])), "Unnamed Group");
        assert_eq!(display_name(&chat_of("other", None, &[])), "Unknown Chat");
    }

    #[test]
    fn others_drops_only_the_signed_in_user() {
        let mut me = member("Sam Lee");
        me.user_id = Some("me-id".to_owned());
        let mut namesake = member("Sam Lee");
        namesake.user_id = Some("other-id".to_owned());
        let outside = member("Guest");

        let kept: Vec<Option<String>> = others(vec![me, namesake, outside], Some("me-id"))
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        assert_eq!(kept, [Some("other-id".to_owned()), None]);
    }

    #[test]
    fn others_keeps_everyone_without_a_profile() {
        assert_eq!(others(vec![member("A"), member("B")], None).len(), 2);
    }

    #[test]
    fn members_parse_from_graph_json() {
        let page: Page<ChatMember> = serde_json::from_str(
            r##"{"value":[{"@odata.type":"#microsoft.graph.aadUserConversationMember","id":"MCMj","roles":["owner"],
                "displayName":"Ada Lovelace","userId":"8b081ef6","email":"ada@example.com","tenantId":"t"}]}"##,
        )
        .unwrap();
        let member = page.value.first().unwrap();
        assert_eq!(member.user_id.as_deref(), Some("8b081ef6"));
        assert_eq!(member.id.as_deref(), Some("MCMj"));
    }

    #[test]
    fn message_accessors_fall_back_cleanly() {
        let message: Message = serde_json::from_str(
            r#"{"id":"1","createdDateTime":"2026-01-01T00:00:00Z","from":null,"body":{"content":"<p>hi</p>"},
                "attachments":[{"id":"a","name":"x.pdf","contentType":null}]}"#,
        )
        .unwrap();
        assert_eq!(message.sender_name(), "Unknown");
        assert_eq!(message.content(), "<p>hi</p>");
        assert_eq!(
            message.attachment("a").and_then(|a| a.name.as_deref()),
            Some("x.pdf")
        );
        assert!(message.attachment("b").is_none());
    }

    #[test]
    fn chats_parse_from_graph_json() {
        let page: Page<Chat> = serde_json::from_str(
            r#"{"value":[{"id":"19:x","topic":null,"chatType":"oneOnOne","lastUpdatedDateTime":"2026-01-01T00:00:00Z"}]}"#,
        )
        .unwrap();
        let chat = page.value.first().unwrap();
        assert_eq!(chat.kind, "oneOnOne");
        assert!(chat.members.is_empty());
    }
}
