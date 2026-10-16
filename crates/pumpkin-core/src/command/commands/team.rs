use std::sync::{MutexGuard, PoisonError};

use crate::command::argument_builder::{
    ArgumentBuilder, LiteralArgumentBuilder, RequiredArgumentBuilder, argument, command, literal,
};
use crate::command::argument_types::component::ComponentArgumentType;
use crate::command::argument_types::core::bool::BoolArgumentType;
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::score_holder::{ResolvedScoreHolder, ScoreHolderArgumentType};
use crate::command::argument_types::team::TeamArgumentType;
use crate::command::argument_types::team_color::TeamColorArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::response_tracker::CommandResponseTracker;
use crate::world::scoreboard::{CollisionRule, NameTagVisibility, Scoreboard, Team};
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;
use pumpkin_util::text::color::{Color, NamedColor};

const DESCRIPTION: &str = "Manages teams.";
const PERMISSION: &str = "minecraft:command.team";

const ARG_TEAM: &str = "team";
const ARG_DISPLAY_NAME: &str = "displayName";
const ARG_MEMBERS: &str = "members";
const ARG_VALUE: &str = "value";
const ARG_PREFIX: &str = "prefix";
const ARG_SUFFIX: &str = "suffix";
const ARG_ALLOWED: &str = "allowed";

const ERROR_TEAM_ALREADY_EXISTS: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_ADD_DUPLICATE,
    translation::java::COMMANDS_TEAM_ADD_DUPLICATE,
);
const ERROR_TEAM_ALREADY_EMPTY: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_EMPTY_UNCHANGED,
    translation::java::COMMANDS_TEAM_EMPTY_UNCHANGED,
);
const ERROR_TEAM_ALREADY_NAME: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_NAME_UNCHANGED,
    translation::java::COMMANDS_TEAM_OPTION_NAME_UNCHANGED,
);
const ERROR_TEAM_ALREADY_COLOR: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_COLOR_UNCHANGED,
    translation::java::COMMANDS_TEAM_OPTION_COLOR_UNCHANGED,
);
const ERROR_TEAM_ALREADY_FRIENDLYFIRE_ENABLED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_ALREADYENABLED,
    translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_ALREADYENABLED,
);
const ERROR_TEAM_ALREADY_FRIENDLYFIRE_DISABLED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_ALREADYDISABLED,
    translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_ALREADYDISABLED,
);
const ERROR_TEAM_ALREADY_FRIENDLYINVISIBLES_ENABLED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_ALREADYENABLED,
    translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_ALREADYENABLED,
);
const ERROR_TEAM_ALREADY_FRIENDLYINVISIBLES_DISABLED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_ALREADYDISABLED,
    translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_ALREADYDISABLED,
);
const ERROR_TEAM_NAMETAG_VISIBLITY_UNCHANGED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_NAMETAGVISIBILITY_UNCHANGED,
    translation::java::COMMANDS_TEAM_OPTION_NAMETAGVISIBILITY_UNCHANGED,
);
const ERROR_TEAM_DEATH_MESSAGE_VISIBLITY_UNCHANGED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_DEATHMESSAGEVISIBILITY_UNCHANGED,
    translation::java::COMMANDS_TEAM_OPTION_DEATHMESSAGEVISIBILITY_UNCHANGED,
);
const ERROR_TEAM_COLLISION_UNCHANGED: CommandErrorType<0> = CommandErrorType::new(
    translation::java::COMMANDS_TEAM_OPTION_COLLISIONRULE_UNCHANGED,
    translation::java::COMMANDS_TEAM_OPTION_COLLISIONRULE_UNCHANGED,
);

fn translatable(key: &'static str, args: impl Into<Vec<TextComponent>>) -> TextComponent {
    TextComponent::translate_cross(key, key, args)
}

fn count(value: usize) -> TextComponent {
    TextComponent::text(value.to_string())
}

fn lock_scoreboard<'a>(context: &'a CommandContext) -> MutexGuard<'a, Scoreboard> {
    context
        .world()
        .scoreboard
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Applies `change` to a copy of the team and sends the copy out, like a `PlayerTeam` setter
/// calling `onTeamChanged`. Returns the formatted display name after the change.
fn change_team(
    context: &CommandContext,
    scoreboard: &mut Scoreboard,
    team: &Team,
    change: impl FnOnce(&mut Team),
) -> TextComponent {
    let mut team = team.clone();
    change(&mut team);
    let display_name = team.formatted_display_name();
    scoreboard.update_team(context.world().as_ref(), team);
    display_name
}

struct ListTeamsExecutor;

impl CommandExecutor for ListTeamsExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let scoreboard = lock_scoreboard(context);
        let teams: Vec<TextComponent> = scoreboard
            .get_player_teams()
            .map(Team::formatted_display_name)
            .collect();
        drop(scoreboard);
        let size = teams.len();
        if teams.is_empty() {
            context.source.send_feedback(
                translatable(translation::java::COMMANDS_TEAM_LIST_TEAMS_EMPTY, []),
                false,
            );
        } else {
            context.source.send_feedback(
                translatable(
                    translation::java::COMMANDS_TEAM_LIST_TEAMS_SUCCESS,
                    [count(size), TextComponent::join_with_comma(teams)],
                ),
                false,
            );
        }
        Ok(size as i32)
    }
}

struct ListMembersExecutor;

impl CommandExecutor for ListMembersExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?;
        let display_name = team.formatted_display_name();
        let mut members = team.players.clone();
        drop(scoreboard);
        let size = members.len();
        if members.is_empty() {
            context.source.send_feedback(
                translatable(
                    translation::java::COMMANDS_TEAM_LIST_MEMBERS_EMPTY,
                    [display_name],
                ),
                false,
            );
        } else {
            // ComponentUtils.formatList(Collection<String>) sorts with String.compareTo.
            members.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let names = members
                .into_iter()
                .map(|name| TextComponent::text(name).color(Color::Named(NamedColor::Green)))
                .collect();
            context.source.send_feedback(
                translatable(
                    translation::java::COMMANDS_TEAM_LIST_MEMBERS_SUCCESS,
                    [
                        display_name,
                        count(size),
                        TextComponent::join_with_comma(names),
                    ],
                ),
                false,
            );
        }
        Ok(size as i32)
    }
}

struct CreateTeamExecutor {
    has_display_name: bool,
}

impl CommandExecutor for CreateTeamExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let name = StringArgumentType::get(context, ARG_TEAM)?.to_string();
        let display_name = if self.has_display_name {
            ComponentArgumentType::get(context, ARG_DISPLAY_NAME)?
        } else {
            TextComponent::text(name.clone())
        };
        let mut scoreboard = lock_scoreboard(context);
        if scoreboard.get_team(&name).is_some() {
            return Err(ERROR_TEAM_ALREADY_EXISTS.create_without_context());
        }

        let mut team = Team::new(name);
        team.display_name = display_name;
        let formatted = team.formatted_display_name();
        scoreboard.add_team(context.world().as_ref(), team);
        let size = scoreboard.get_teams().len();
        drop(scoreboard);
        context.source.send_feedback(
            translatable(translation::java::COMMANDS_TEAM_ADD_SUCCESS, [formatted]),
            true,
        );
        Ok(size as i32)
    }
}

struct DeleteTeamExecutor;

impl CommandExecutor for DeleteTeamExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?;
        let formatted = team.formatted_display_name();
        let name = team.name.clone();
        scoreboard.remove_team(context.world().as_ref(), &name);
        let size = scoreboard.get_teams().len();
        drop(scoreboard);
        context.source.send_feedback(
            translatable(translation::java::COMMANDS_TEAM_REMOVE_SUCCESS, [formatted]),
            true,
        );
        Ok(size as i32)
    }
}

struct EmptyTeamExecutor;

impl CommandExecutor for EmptyTeamExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?;
        let members = team.players.clone();
        if members.is_empty() {
            return Err(ERROR_TEAM_ALREADY_EMPTY.create_without_context());
        }
        let name = team.name.clone();
        let formatted = team.formatted_display_name();
        for member in &members {
            scoreboard.remove_player_from_team(context.world().as_ref(), &name, member);
        }
        drop(scoreboard);
        context.source.send_feedback(
            translatable(
                translation::java::COMMANDS_TEAM_EMPTY_SUCCESS,
                [count(members.len()), formatted],
            ),
            true,
        );
        Ok(members.len() as i32)
    }
}

struct JoinTeamExecutor {
    has_members: bool,
}

impl CommandExecutor for JoinTeamExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?;
        let team_name = team.name.clone();
        let formatted = team.formatted_display_name();
        drop(scoreboard);
        let members = if self.has_members {
            ScoreHolderArgumentType::get_score_holders(context, ARG_MEMBERS)?
        } else {
            vec![ResolvedScoreHolder::entity(
                context.source.entity_or_err()?.as_ref(),
            )]
        };

        scoreboard = lock_scoreboard(context);
        let mut tracker = CommandResponseTracker::default();
        for member in members {
            let added = scoreboard.add_player_to_team(
                context.world().as_ref(),
                &team_name,
                member.name.clone(),
            );
            tracker.track(member, added);
        }
        drop(scoreboard);
        Ok(tracker.send_feedback(
            &context.source,
            true,
            |holder, _| {
                translatable(
                    translation::java::COMMANDS_TEAM_JOIN_SUCCESS_SINGLE,
                    [holder.display_name, formatted.clone()],
                )
            },
            |holder_count, _| {
                translatable(
                    translation::java::COMMANDS_TEAM_JOIN_SUCCESS_MULTIPLE,
                    [
                        TextComponent::text(holder_count.to_string()),
                        formatted.clone(),
                    ],
                )
            },
        ))
    }
}

struct LeaveTeamExecutor;

impl CommandExecutor for LeaveTeamExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let members = ScoreHolderArgumentType::get_score_holders(context, ARG_MEMBERS)?;
        let mut scoreboard = lock_scoreboard(context);
        let mut tracker = CommandResponseTracker::default();
        for member in members {
            let removed =
                scoreboard.remove_player_from_any_team(context.world().as_ref(), &member.name);
            tracker.track(member, removed);
        }
        drop(scoreboard);
        Ok(tracker.send_feedback(
            &context.source,
            true,
            |holder, _| {
                translatable(
                    translation::java::COMMANDS_TEAM_LEAVE_SUCCESS_SINGLE,
                    [holder.display_name],
                )
            },
            |holder_count, _| {
                translatable(
                    translation::java::COMMANDS_TEAM_LEAVE_SUCCESS_MULTIPLE,
                    [TextComponent::text(holder_count.to_string())],
                )
            },
        ))
    }
}

struct SetDisplayNameExecutor;

impl CommandExecutor for SetDisplayNameExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let display_name = ComponentArgumentType::get(context, ARG_DISPLAY_NAME)?;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if team.display_name == display_name {
            return Err(ERROR_TEAM_ALREADY_NAME.create_without_context());
        }
        let formatted = change_team(context, &mut scoreboard, &team, |team| {
            team.display_name = display_name;
        });
        drop(scoreboard);
        context.source.send_feedback(
            translatable(
                translation::java::COMMANDS_TEAM_OPTION_NAME_SUCCESS,
                [formatted],
            ),
            true,
        );
        Ok(0)
    }
}

/// Vanilla `setColor` with a color, `clearColor` without one.
struct SetColorExecutor {
    reset: bool,
}

impl CommandExecutor for SetColorExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let color = if self.reset {
            None
        } else {
            Some(TeamColorArgumentType::get(context, ARG_VALUE)?)
        };
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if team.color == color {
            return Err(ERROR_TEAM_ALREADY_COLOR.create_without_context());
        }
        let formatted = change_team(context, &mut scoreboard, &team, |team| team.color = color);
        drop(scoreboard);
        let message = match color {
            Some(color) => translatable(
                translation::java::COMMANDS_TEAM_OPTION_COLOR_SUCCESS,
                [formatted, TextComponent::text(color.name())],
            ),
            None => translatable(
                translation::java::COMMANDS_TEAM_OPTION_COLOR_CLEAR_SUCCESS,
                [formatted],
            ),
        };
        context.source.send_feedback(message, true);
        Ok(0)
    }
}

/// Vanilla `setFriendlyFire` and `setFriendlySight`, which flip one bit of the team's options.
struct SetOptionExecutor {
    bit: i8,
}

impl CommandExecutor for SetOptionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let allowed = BoolArgumentType::get(context, ARG_ALLOWED)?;
        let friendly_fire = self.bit == Team::BIT_FRIENDLY_FIRE;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if (team.options & self.bit != 0) == allowed {
            let error: &'static CommandErrorType<0> = match (friendly_fire, allowed) {
                (true, true) => &ERROR_TEAM_ALREADY_FRIENDLYFIRE_ENABLED,
                (true, false) => &ERROR_TEAM_ALREADY_FRIENDLYFIRE_DISABLED,
                (false, true) => &ERROR_TEAM_ALREADY_FRIENDLYINVISIBLES_ENABLED,
                (false, false) => &ERROR_TEAM_ALREADY_FRIENDLYINVISIBLES_DISABLED,
            };
            return Err(error.create_without_context());
        }
        let bit = self.bit;
        let formatted = change_team(context, &mut scoreboard, &team, |team| {
            if allowed {
                team.options |= bit;
            } else {
                team.options &= !bit;
            }
        });
        drop(scoreboard);
        let key = match (friendly_fire, allowed) {
            (true, true) => translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_ENABLED,
            (true, false) => translation::java::COMMANDS_TEAM_OPTION_FRIENDLYFIRE_DISABLED,
            (false, true) => translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_ENABLED,
            (false, false) => {
                translation::java::COMMANDS_TEAM_OPTION_SEEFRIENDLYINVISIBLES_DISABLED
            }
        };
        context
            .source
            .send_feedback(translatable(key, [formatted]), true);
        Ok(0)
    }
}

struct SetNameTagVisibilityExecutor(NameTagVisibility);

impl CommandExecutor for SetNameTagVisibilityExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let visibility = self.0;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if team.nametag_visibility == visibility {
            return Err(ERROR_TEAM_NAMETAG_VISIBLITY_UNCHANGED.create_without_context());
        }
        let formatted = change_team(context, &mut scoreboard, &team, |team| {
            team.nametag_visibility = visibility;
        });
        drop(scoreboard);
        context.source.send_feedback(
            translatable(
                translation::java::COMMANDS_TEAM_OPTION_NAMETAGVISIBILITY_SUCCESS,
                [formatted, visibility.display_name()],
            ),
            true,
        );
        Ok(0)
    }
}

struct SetDeathMessageVisibilityExecutor(NameTagVisibility);

impl CommandExecutor for SetDeathMessageVisibilityExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let visibility = self.0;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if team.death_message_visibility == visibility {
            return Err(ERROR_TEAM_DEATH_MESSAGE_VISIBLITY_UNCHANGED.create_without_context());
        }
        let formatted = change_team(context, &mut scoreboard, &team, |team| {
            team.death_message_visibility = visibility;
        });
        drop(scoreboard);
        context.source.send_feedback(
            translatable(
                translation::java::COMMANDS_TEAM_OPTION_DEATHMESSAGEVISIBILITY_SUCCESS,
                [formatted, visibility.display_name()],
            ),
            true,
        );
        Ok(0)
    }
}

struct SetCollisionExecutor(CollisionRule);

impl CommandExecutor for SetCollisionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let collision = self.0;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        if team.collision_rule == collision {
            return Err(ERROR_TEAM_COLLISION_UNCHANGED.create_without_context());
        }
        let formatted = change_team(context, &mut scoreboard, &team, |team| {
            team.collision_rule = collision;
        });
        drop(scoreboard);
        context.source.send_feedback(
            translatable(
                translation::java::COMMANDS_TEAM_OPTION_COLLISIONRULE_SUCCESS,
                [formatted, collision.display_name()],
            ),
            true,
        );
        Ok(0)
    }
}

/// Vanilla `setPrefix` and `setSuffix`.
struct SetAffixExecutor {
    prefix: bool,
}

impl CommandExecutor for SetAffixExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let (argument, key) = if self.prefix {
            (
                ARG_PREFIX,
                translation::java::COMMANDS_TEAM_OPTION_PREFIX_SUCCESS,
            )
        } else {
            (
                ARG_SUFFIX,
                translation::java::COMMANDS_TEAM_OPTION_SUFFIX_SUCCESS,
            )
        };
        let affix = ComponentArgumentType::get(context, argument)?;
        let mut scoreboard = lock_scoreboard(context);
        let team = TeamArgumentType::get_team(context, ARG_TEAM, &scoreboard)?.clone();
        let prefix = self.prefix;
        change_team(context, &mut scoreboard, &team, |team| {
            if prefix {
                team.player_prefix = affix.clone();
            } else {
                team.player_suffix = affix.clone();
            }
        });
        drop(scoreboard);
        context
            .source
            .send_feedback(translatable(key, [affix]), false);
        Ok(1)
    }
}

fn team_argument() -> RequiredArgumentBuilder {
    argument(ARG_TEAM, TeamArgumentType)
}

fn list_branch() -> LiteralArgumentBuilder {
    literal("list")
        .executes(ListTeamsExecutor)
        .then(team_argument().executes(ListMembersExecutor))
}

fn add_branch() -> LiteralArgumentBuilder {
    literal("add").then(
        argument(ARG_TEAM, StringArgumentType::SingleWord)
            .executes(CreateTeamExecutor {
                has_display_name: false,
            })
            .then(
                argument(ARG_DISPLAY_NAME, ComponentArgumentType).executes(CreateTeamExecutor {
                    has_display_name: true,
                }),
            ),
    )
}

fn join_branch() -> LiteralArgumentBuilder {
    literal("join").then(
        team_argument()
            .executes(JoinTeamExecutor { has_members: false })
            .then(
                argument(ARG_MEMBERS, ScoreHolderArgumentType::Multiple)
                    .executes(JoinTeamExecutor { has_members: true }),
            ),
    )
}

fn leave_branch() -> LiteralArgumentBuilder {
    literal("leave")
        .then(argument(ARG_MEMBERS, ScoreHolderArgumentType::Multiple).executes(LeaveTeamExecutor))
}

fn visibility_branch<E: CommandExecutor + 'static>(
    name: &'static str,
    executor: fn(NameTagVisibility) -> E,
) -> LiteralArgumentBuilder {
    literal(name)
        .then(literal("never").executes(executor(NameTagVisibility::Never)))
        .then(literal("hideForOtherTeams").executes(executor(NameTagVisibility::HideForOtherTeams)))
        .then(literal("hideForOwnTeam").executes(executor(NameTagVisibility::HideForOwnTeam)))
        .then(literal("always").executes(executor(NameTagVisibility::Always)))
}

fn modify_branch() -> LiteralArgumentBuilder {
    literal("modify").then(
        team_argument()
            .then(literal("displayName").then(
                argument(ARG_DISPLAY_NAME, ComponentArgumentType).executes(SetDisplayNameExecutor),
            ))
            .then(
                literal("color")
                    .then(literal("reset").executes(SetColorExecutor { reset: true }))
                    .then(
                        argument(ARG_VALUE, TeamColorArgumentType)
                            .executes(SetColorExecutor { reset: false }),
                    ),
            )
            .then(
                literal("friendlyFire").then(argument(ARG_ALLOWED, BoolArgumentType).executes(
                    SetOptionExecutor {
                        bit: Team::BIT_FRIENDLY_FIRE,
                    },
                )),
            )
            .then(literal("seeFriendlyInvisibles").then(
                argument(ARG_ALLOWED, BoolArgumentType).executes(SetOptionExecutor {
                    bit: Team::BIT_SEE_INVISIBLES,
                }),
            ))
            .then(visibility_branch(
                "nametagVisibility",
                SetNameTagVisibilityExecutor,
            ))
            .then(visibility_branch(
                "deathMessageVisibility",
                SetDeathMessageVisibilityExecutor,
            ))
            .then(
                literal("collisionRule")
                    .then(literal("never").executes(SetCollisionExecutor(CollisionRule::Never)))
                    .then(
                        literal("pushOwnTeam")
                            .executes(SetCollisionExecutor(CollisionRule::PushOwnTeam)),
                    )
                    .then(
                        literal("pushOtherTeams")
                            .executes(SetCollisionExecutor(CollisionRule::PushOtherTeams)),
                    )
                    .then(literal("always").executes(SetCollisionExecutor(CollisionRule::Always))),
            )
            .then(
                literal("prefix").then(
                    argument(ARG_PREFIX, ComponentArgumentType)
                        .executes(SetAffixExecutor { prefix: true }),
                ),
            )
            .then(
                literal("suffix").then(
                    argument(ARG_SUFFIX, ComponentArgumentType)
                        .executes(SetAffixExecutor { prefix: false }),
                ),
            ),
    )
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("team", DESCRIPTION)
            .requires(PERMISSION)
            .then(list_branch())
            .then(add_branch())
            .then(literal("remove").then(team_argument().executes(DeleteTeamExecutor)))
            .then(literal("empty").then(team_argument().executes(EmptyTeamExecutor)))
            .then(join_branch())
            .then(leave_branch())
            .then(modify_branch()),
    );
}
