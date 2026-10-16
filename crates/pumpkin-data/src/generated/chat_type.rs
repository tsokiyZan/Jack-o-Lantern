/* This file is generated. Do not edit manually. */
#[doc = "Vanilla `ChatTypeDecoration.Parameter`: what fills each argument of the translation."]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ChatTypeParameter {
    Sender,
    Target,
    Content,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u32)]
pub enum ChatType {
    Chat,
    EmoteCommand,
    MsgCommandIncoming,
    MsgCommandOutgoing,
    SayCommand,
    TeamMsgCommandIncoming,
    TeamMsgCommandOutgoing,
}
impl ChatType {
    #[doc = "Returns the chat type from a resource name (bare or namespaced)."]
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "minecraft:chat" | "chat" => Some(Self::Chat),
            "minecraft:emote_command" | "emote_command" => Some(Self::EmoteCommand),
            "minecraft:msg_command_incoming" | "msg_command_incoming" => {
                Some(Self::MsgCommandIncoming)
            }
            "minecraft:msg_command_outgoing" | "msg_command_outgoing" => {
                Some(Self::MsgCommandOutgoing)
            }
            "minecraft:say_command" | "say_command" => Some(Self::SayCommand),
            "minecraft:team_msg_command_incoming" | "team_msg_command_incoming" => {
                Some(Self::TeamMsgCommandIncoming)
            }
            "minecraft:team_msg_command_outgoing" | "team_msg_command_outgoing" => {
                Some(Self::TeamMsgCommandOutgoing)
            }
            _ => None,
        }
    }
    #[doc = "Returns the chat type with this numeric ID in the synced registry."]
    #[must_use]
    pub const fn from_id(id: u32) -> Option<Self> {
        match id {
            0u32 => Some(Self::Chat),
            1u32 => Some(Self::EmoteCommand),
            2u32 => Some(Self::MsgCommandIncoming),
            3u32 => Some(Self::MsgCommandOutgoing),
            4u32 => Some(Self::SayCommand),
            5u32 => Some(Self::TeamMsgCommandIncoming),
            6u32 => Some(Self::TeamMsgCommandOutgoing),
            _ => None,
        }
    }
    #[doc = "Returns the numeric ID of the chat type in the synced registry."]
    #[must_use]
    pub const fn id(&self) -> u32 {
        *self as u32
    }
    #[doc = "Returns the bare string name of the chat type."]
    #[must_use]
    pub const fn to_name(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::EmoteCommand => "emote_command",
            Self::MsgCommandIncoming => "msg_command_incoming",
            Self::MsgCommandOutgoing => "msg_command_outgoing",
            Self::SayCommand => "say_command",
            Self::TeamMsgCommandIncoming => "team_msg_command_incoming",
            Self::TeamMsgCommandOutgoing => "team_msg_command_outgoing",
        }
    }
    #[doc = "Returns the fully-qualified asset id of the chat type."]
    #[must_use]
    pub const fn asset_id(&self) -> &'static str {
        match self {
            Self::Chat => "minecraft:chat",
            Self::EmoteCommand => "minecraft:emote_command",
            Self::MsgCommandIncoming => "minecraft:msg_command_incoming",
            Self::MsgCommandOutgoing => "minecraft:msg_command_outgoing",
            Self::SayCommand => "minecraft:say_command",
            Self::TeamMsgCommandIncoming => "minecraft:team_msg_command_incoming",
            Self::TeamMsgCommandOutgoing => "minecraft:team_msg_command_outgoing",
        }
    }
    #[doc = "Returns the translation key for formatting chat messages of this type."]
    #[must_use]
    pub const fn translation_key(&self) -> &'static str {
        match self {
            Self::Chat => "chat.type.text",
            Self::EmoteCommand => "chat.type.emote",
            Self::MsgCommandIncoming => "commands.message.display.incoming",
            Self::MsgCommandOutgoing => "commands.message.display.outgoing",
            Self::SayCommand => "chat.type.announcement",
            Self::TeamMsgCommandIncoming => "chat.type.team.text",
            Self::TeamMsgCommandOutgoing => "chat.type.team.sent",
        }
    }
    #[doc = "Returns the parameters of the chat decoration's translation, in order."]
    #[must_use]
    pub const fn chat_parameters(&self) -> &'static [ChatTypeParameter] {
        match self {
            Self::Chat => &[ChatTypeParameter::Sender, ChatTypeParameter::Content],
            Self::EmoteCommand => &[ChatTypeParameter::Sender, ChatTypeParameter::Content],
            Self::MsgCommandIncoming => &[ChatTypeParameter::Sender, ChatTypeParameter::Content],
            Self::MsgCommandOutgoing => &[ChatTypeParameter::Target, ChatTypeParameter::Content],
            Self::SayCommand => &[ChatTypeParameter::Sender, ChatTypeParameter::Content],
            Self::TeamMsgCommandIncoming => &[
                ChatTypeParameter::Target,
                ChatTypeParameter::Sender,
                ChatTypeParameter::Content,
            ],
            Self::TeamMsgCommandOutgoing => &[
                ChatTypeParameter::Target,
                ChatTypeParameter::Sender,
                ChatTypeParameter::Content,
            ],
        }
    }
    #[doc = "Returns all vanilla chat types."]
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Chat,
            Self::EmoteCommand,
            Self::MsgCommandIncoming,
            Self::MsgCommandOutgoing,
            Self::SayCommand,
            Self::TeamMsgCommandIncoming,
            Self::TeamMsgCommandOutgoing,
        ]
    }
}
