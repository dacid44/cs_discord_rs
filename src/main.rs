#![deny(unused_must_use)]

use std::borrow::Borrow;
use std::collections::HashSet;
use std::path::Path;

use dotenv::dotenv;
use futures::future::join_all;
use itertools::Itertools;
use lazy_static::lazy_static;
// use poise::serenity_prelude as p_serenity;
use mongodb::bson::doc;
use mongodb::Client;
use seq_macro::seq;
use serenity::async_trait;
use serenity::builder::{CreateActionRow, CreateComponents, CreateSelectMenuOption};
use serenity::client::Context as SContext;
use serenity::http::CacheHttp;
use serenity::model::application::component::ActionRowComponent;
use serenity::model::application::interaction::Interaction;
use serenity::model::channel::{Channel, ChannelType, GuildChannel};
use serenity::model::guild::{Member, Role};
use serenity::model::id::{GuildId, RoleId};
use serenity::model::mention::Mention;
use serenity::model::prelude::component::{ButtonStyle, ComponentType};
use serenity::prelude::*;
use serenity::utils::MessageBuilder;
use thiserror::Error;
use tokio::sync::OnceCell;

use crate::ClassError::InvalidChannelType;
use crate::classes::{Class, Server};

mod classes;

// const IS_DEV: bool = true;

lazy_static! {
    static ref ENV: EnvVars = EnvVars::init().unwrap();
}

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;
struct Data {}

struct EnvVars {
    bot_token: String,
    guild_id: u64,
    mongodb_name: String,
    mongodb_user: String,
    mongodb_password: String,
}

impl EnvVars {
    fn init() -> Result<Self, Error> {
        use std::env::var;
        // use std::env::VarError;

        // fn get_var(name: &str) -> Result<String, VarError> {
        //     if IS_DEV {
        //         var(format!("DEV_{}", name))
        //     } else {
        //         var(name)
        //     }
        // }

        if Path::new(".env").exists() {
            dotenv()?;
        }

        Ok(Self {
            bot_token: var("BOT_TOKEN")?,
            guild_id: var("GUILD_ID")?.parse::<u64>()?,
            mongodb_name: var("MONGODB_NAME")?,
            mongodb_user: var("MONGODB_USER")?,
            mongodb_password: var("MONGODB_PASSWORD")?,
        })
    }
}

static MONGODB_CONN: OnceCell<Client> = OnceCell::const_new();

async fn get_conn() -> Client {
    MONGODB_CONN
        .get_or_init(|| async {
            Client::with_uri_str(format!(
                "mongodb+srv://{}:{}@cs-discord.kev09.mongodb.net/?retryWrites=true&w=majority",
                ENV.mongodb_user, ENV.mongodb_password,
            ))
            .await
            .expect("Failed to connect to Mongo server.")
        })
        .await
        .clone()
}

#[tokio::main]
async fn main() {
    println!("Hello, world!");

    let commands = vec![echo(), register(), class(), config()];
    let create_commands = poise::builtins::create_application_commands(&commands);

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands,
            ..Default::default()
        })
        .token(&ENV.bot_token)
        .intents(GatewayIntents::non_privileged() | GatewayIntents::MESSAGE_CONTENT)
        .client_settings(|c| c.event_handler(Handler))
        // .client_settings(|c| c
        //     .event_handler(ClassMenuButtonHandler)
        //     .event_handler(ClassMenuHandler)
        // )
        .user_data_setup(move |ctx, _ready, _framework| {
            Box::pin(async move {
                GuildId(ENV.guild_id)
                    .set_application_commands(ctx.http(), |b| {
                        *b = create_commands;
                        b
                    })
                    .await
                    .expect("Error registering guild commands");

                Ok(Data {})
            })
        })
        .build()
        .await
        .expect("Error building poise framework");

    framework.start().await.unwrap();

    // p_serenity::GuildId(ENV.guild_id).set_application_commands(
    //     framework.client().cache_and_http.http(),
    //     |b| { *b = create_commands; b }
    // ).await.expect("Error registering guild commands");
}

#[poise::command(prefix_command)]
async fn register(ctx: Context<'_>) -> Result<(), Error> {
    poise::builtins::register_application_commands_buttons(ctx).await?;
    Ok(())
}

#[poise::command(slash_command)]
async fn echo(context: Context<'_>, text: String) -> Result<(), Error> {
    context.say(format!("{}{}", &text, text)).await?;
    Ok(())
}

// macro_rules! repeat_arg {
//     ($name:ident: $type:ty, $num:expr) => { $name$num: $type };
//     ($name:ident: $type:ty, $num:expr, $($nums:expr),+) => { $name$num: $type, repeat_arg!($name: $type, $num $($nums),+) };
// }

#[poise::command(
    slash_command,
    subcommands(
        "ClassCommand::info",
        "ClassCommand::list",
        "ClassCommand::create",
        "ClassCommand::track",
        "ClassCommand::untrack",
        "ClassCommand::delete",
        "ClassCommand::menu",
    )
)]
async fn class(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}
struct ClassCommand;
impl ClassCommand {
    #[poise::command(
        slash_command,
        ephemeral,
    )]
    async fn list(ctx: Context<'_>, mention: Option<bool>) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        let mention = mention.unwrap_or(false);
        let classes = Class::list(ctx.guild().ok_or(ClassError::NoServer)?.id).await?;

        if classes.is_empty() {
            ctx.say("No classes found for this server.").await?;
            return Ok(());
        }

        ctx.say(format!(
            "Found {} classes: {}",
            classes.len(),
            classes.into_iter()
                .sorted_by(|c1, c2| human_sort::compare(&c1.name, &c2.name))
                .map(|c| if mention { c.role.mention().to_string() } else { c.name })
                .join(", ")
        )).await?;

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
    )]
    async fn info(ctx: Context<'_>, class: Role, mention: Option<bool>) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        let mention = mention.unwrap_or(false);
        let guild = ctx.guild().ok_or(ClassError::NoServer)?;
        let role = class;
        let class = Class::find_by_role(role.id).await?.ok_or(ClassError::InvalidClass)?;

        let message = format!(
            r#"
Name: \"{}\",
Short name: \"{}\",
Role: {},
Category: `{}`,
Text Channels: {},
Voice Channels: {},
"#,
            class.name,
            class.short_name,
            if mention {
                class.role.mention().to_string()
            } else {
                format!("`{}`", role.name)
            },
            guild.channels.get(&class.category)
                .ok_or_else(|| ClassError::InvalidChannel(class.category.mention()))
                .and_then(|c| match c {
                    Channel::Category(cc) => Ok(cc.name()),
                    _ => Err(ClassError::InvalidChannelType(class.category.mention())),
                })?,
            class.text_channels.iter()
                .map(|c| c.mention())
                .join(", "),
            class.voice_channels.iter()
                .map(|c| c.mention())
                .join(", "),
        );

        ctx.say(
            MessageBuilder::new()
                .push_bold("Class info:")
                .quote_rest()
                .push(&message[1..])
                .build()
        ).await?;

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
        required_bot_permissions = "MANAGE_GUILD",
    )]
    async fn create(ctx: Context<'_>, name: String) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        Class::create(ctx, &name).await?;

        ctx.say(format!("Created new class \"{}\"", name)).await?;

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
    )]
    #[allow(clippy::too_many_arguments, clippy::vec_init_then_push)]
    async fn track(
        ctx: Context<'_>,
        name: Option<String>,
        role: Role,
        #[channel_types("Category")] category: Channel,
        // This is really, really stupid, I know. It doesn't seem like this can be done with a macro, either.
        #[channel_types("Text", "Voice")] channel1: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel2: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel3: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel4: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel5: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel6: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel7: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel8: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel9: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel10: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel11: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel12: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel13: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel14: Option<GuildChannel>,
        #[channel_types("Text", "Voice")] channel15: Option<GuildChannel>,
    ) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        let mut channels = Vec::new();
        seq!(N in 1..=15 {
            channels.push(channel~N);
        });
        let channels = channels.into_iter().flatten().collect::<Vec<_>>();

        let category = if let Channel::Category(c) = category {
            c
        } else {
            return Err(ClassError::InvalidChannelType(category.mention()))?;
        };

        let class = Class::track(ctx, name, role, category, &channels).await?;

        ctx.say(format!("Now tracking class \"{}\"", class.name)).await?;

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
    )]
    async fn untrack(ctx: Context<'_>, class: Role) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        if let Some(name) = Class::find_by_role(class.id)
            .await?
            .ok_or(ClassError::InvalidClass)?
            .untrack()
            .await?
        {
            ctx.say(format!("No longer tracking class {}.", name)).await?;
        } else {
            Err(ClassError::InvalidClass)?;
        }

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
        required_bot_permissions = "MANAGE_GUILD",
    )]
    async fn delete(ctx: Context<'_>, class: Role) -> Result<(), Error> {
        ctx.defer_ephemeral().await?;

        let (result, errors) = Class::find_by_role(class.id)
            .await?
            .ok_or(ClassError::InvalidClass)?
            .delete(ctx)
            .await?;

        if let Some(name) = result {
            ctx.say(format!("Deleted class \"{}\".", name)).await?;
        } else {
            ctx.say("Failed to delete the class.").await?;
        }

        if !errors.is_empty() {
            ctx.say(format!("Errors: {:?}", errors)).await?;
        }

        Ok(())
    }

    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
    )]
    async fn menu(ctx: Context<'_>, #[channel_types("Text")] channel: Option<GuildChannel>) -> Result<(), Error> {
        let guild = ctx.guild().ok_or(ClassError::NoServer)?;
        let channel = channel.unwrap_or(
            guild.channels.get(&ctx.channel_id())
                .ok_or_else(|| ClassError::InvalidChannel(ctx.channel_id().mention()))
                .and_then(|c| c.clone().guild().ok_or_else(|| InvalidChannelType(c.mention())))?
        );
        if channel.kind != ChannelType::Text {
            Err(ClassError::InvalidChannelType(channel.mention()))?;
        }

        let http = ctx.discord().http();

        channel.send_message(http, |m| m
            .components(|c| c
                .create_action_row(|r| r
                    .create_button(|b| b
                        .custom_id("class_menu_button")
                        .style(ButtonStyle::Primary)
                        .label("Click here to choose classes!")
                        .emoji('📝') // U+1F4DD : MEMO
                    )
                )
            )
        ).await?;

        ctx.say("Done!").await?;

        Ok(())
    }
}

#[poise::command(slash_command, subcommands("ConfigCommand::refrole"))]
async fn config(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}
struct ConfigCommand;
impl ConfigCommand {
    #[poise::command(slash_command, subcommands("ConfigRefroleCommand::set"))]
    async fn refrole(_ctx: Context<'_>) -> Result<(), Error> {
        Ok(())
    }
}

struct ConfigRefroleCommand;
impl ConfigRefroleCommand {
    #[poise::command(
        slash_command,
        ephemeral,
        required_permissions = "MANAGE_GUILD",
        required_bot_permissions = "MANAGE_GUILD",
    )]
    async fn set(ctx: Context<'_>, role: Role) -> Result<(), Error> {
        let mut server = Server::get_or_create(ctx.guild_id().ok_or(ClassError::NoServer)?)
            .await?;
        server
            .set_refrole(ctx, role.id)
            .await?;

        ctx.say(format!("{} is now the refrole for this server.", role.mention())).await?;

        Ok(())
    }
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: SContext, interaction: Interaction) {
        join_all(vec![
            EventHandler::interaction_create(&ClassMenuButtonHandler, ctx.clone(), interaction.clone()),
            EventHandler::interaction_create(&ClassMenuHandler, ctx.clone(), interaction.clone()),
        ]).await;
    }
}

struct ClassMenuButtonHandler;

#[async_trait]
impl EventHandler for ClassMenuButtonHandler {
    async fn interaction_create(&self, ctx: SContext, interaction: Interaction) {
        let component = if let Interaction::MessageComponent(c) = interaction {
            c
        } else {
            return;
        };
        if component.data.component_type != ComponentType::Button || component.data.custom_id != "class_menu_button" {
            return;
        }

        let http = ctx.http();

        // Throw away the result as deferring is not critical
        // component.defer(http).await.ok();

        let member = if let Some(m) = &component.member {
            m
        } else {
            eprintln!("Error handling class_menu_button: {:?}", ClassError::NoServer);
            return;
        };

        let server_id = if let Some(id) = component.guild_id {
            id
        } else {
            eprintln!("Error handling class_menu_button: {:?}", ClassError::NoServer);
            return;
        };

        let menu = match build_class_menu(server_id, member).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error handling class_menu_button: {:?}", e);
                return;
            }
        };

        // Throwing away the error as if the deletion fails, we will know from the error message
        // from creating the new response
        // component.delete_original_interaction_response(http).await.ok();
        if let Err(e) = component.create_interaction_response(http, |r| r.interaction_response_data(|d| d
            .ephemeral(true)
            .set_components(menu)
        )).await {
            eprintln!("Error handling class_menu_button: {:?}", e);
            return;
        }
    }
}

async fn build_class_menu(server_id: GuildId, member: &Member) -> ClassResult<CreateComponents> {
    let member_roles = member.roles.iter().collect::<HashSet<_>>();

    let action_rows = Class::list(server_id).await?
        .iter()
        .sorted_by(|c1, c2| human_sort::compare(&c1.name, &c2.name))
        .map(|c| {
            let mut o = CreateSelectMenuOption::new(&c.name, c.role.to_string());
            o.default_selection(member_roles.contains(&c.role));
            o
        })
        .chunks(25)
        .borrow()
        .into_iter()
        .map(|chunk| chunk.collect::<Vec<_>>())
        .enumerate()
        .map(|(i, chunk)| {
            let mut row = CreateActionRow::default();
            row.create_select_menu(|m| m
                .custom_id(format!("class_menu_button_{}", i))
                .min_values(0)
                .max_values(chunk.len() as u64)
                .options(|o| o.set_options(chunk))
            );
            row
        })
        .collect::<Vec<_>>();

    let mut cc = CreateComponents::default();
    cc.set_action_rows(action_rows);

    Ok(cc)
}

struct ClassMenuHandler;

#[async_trait]
impl EventHandler for ClassMenuHandler {
    async fn interaction_create(&self, ctx: SContext, interaction: Interaction) {
        let component = if let Interaction::MessageComponent(c) = interaction {
            c
        } else {
            return;
        };
        if component.data.component_type != ComponentType::SelectMenu {
            return;
        }

        let custom_id = &*component.data.custom_id;

        let _id = if let Some(id) = parse_class_button_id(custom_id) {
            id
        } else {
            return;
        };

        let http = ctx.http();

        // Throwing away the result as if the defer fails, the user will see an error message
        // regardless of how the error is handled, so we might as well finish handling the input
        component.defer(http).await.ok();

        let member = if let Some(m) = &component.member {
            m
        } else {
            eprintln!("Error handling {}: {:?}", custom_id, ClassError::NoServer);
            return;
        };

        let menu = if let Some(menu) = component.message.components.iter()
            .filter_map(|row| row.components.get(0)
                .and_then(|c| match c {
                    ActionRowComponent::SelectMenu(menu) => Some(menu),
                    _ => None
                })
            )
            .find(|menu| menu.custom_id.as_ref().map(|id| id == custom_id).unwrap_or(false))
        {
            menu
        } else {
            eprintln!("Error handling {}: Could not find matching select menu", custom_id);
            return;
        };

        let member_roles = member.roles.iter().copied().collect::<HashSet<_>>();
        // Unwrapping because this should be a valid role ID
        let menu_roles = menu.options.iter()
            .map(|o| o.value.parse().unwrap())
            .collect::<HashSet<RoleId>>();
        // Unwrapping because this should be a valid role ID
        let new_roles = component.data.values.iter()
            .map(|o| o.parse().unwrap())
            .collect::<HashSet<RoleId>>();

        if let Err(e) = member
            .edit(http, |e| {
                e.roles(&(&member_roles - &menu_roles) | &new_roles)
            })
            .await
        {
            println!(
                "Error handling {}: {:?}", custom_id, ClassError::ApiError(e));
            return;
        }
    }
}

fn parse_class_button_id(id: &str) -> Option<u8> {
    if !id.starts_with("class_menu_button_") {
        return None;
    }

    id[18..].parse().ok()
}

#[derive(Error, Debug)]
pub enum ClassError {
    #[error("There is no refrole set for this server.")]
    NoRefrole,
    #[error("The set refrole for this server is invalid.")]
    InvalidRefrole,
    #[error("Already tracking a class with the given name.")]
    ClassExists,
    #[error("A role with the given name already exists.")]
    RoleExists,
    #[error("A category with the given name already exists.")]
    CategoryExists,
    #[error("This command can only be run inside a server.")]
    NoServer,
    #[error("The given role does not exist in this server.")]
    InvalidRole,
    #[error("The given channel {0} does not exist in this server.")]
    InvalidChannel(Mention),
    #[error("The given channel {0} is of an invalid type.")]
    InvalidChannelType(Mention),
    #[error("The given role is already being used for class {0}.")]
    RoleInUse(String),
    #[error("There is no class assigned to the given role.")]
    InvalidClass,
    #[error("{0}")]
    ApiError(#[from] serenity::Error),
    #[error("{0}")]
    DatabaseError(#[from] mongodb::error::Error),
}

type ClassResult<T> = Result<T, ClassError>;
