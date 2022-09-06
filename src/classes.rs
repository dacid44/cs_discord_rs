use std::collections::HashSet;

use futures::future::TryFutureExt;
use futures::TryStreamExt;
use lazy_static::lazy_static;
use mongodb::Collection;
use mongodb::bson::doc;
use mongodb::options::{DeleteOptions, FindOneAndReplaceOptions, FindOneOptions, FindOptions, Hint};
use serde::{Deserialize, Serialize};
use serenity::http::CacheHttp;
use serenity::model::channel::{Channel, ChannelCategory, ChannelType, GuildChannel, PermissionOverwrite, PermissionOverwriteType};
use serenity::model::guild::Role;
use serenity::model::id::{ChannelId, GuildId, RoleId};
use serenity::model::Permissions;
use serenity::prelude::Mentionable;
use tokio::sync::OnceCell;

use crate::{ClassError, ClassResult, Context, ENV, get_conn};

lazy_static! {
    static ref SERVER_ID_HINT: Hint = Hint::Name("server_id_1".to_string());
    static ref SERVER_ID_NAME_HINT: Hint = Hint::Name("server_id_1_name_1".to_string());
    static ref NAME_HINT: Hint = Hint::Name("name_1".to_string());
    static ref ROLE_HINT: Hint = Hint::Name("role_1".to_string());
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct Server {
    server_id: GuildId,
    admin_roles: Vec<RoleId>,
    refrole: Option<RoleId>,
}

impl Server {

    pub async fn get_or_create(id: GuildId) -> ClassResult<Self> {
        let servers = Self::get_collection().await;

        if let Some(server) = servers
            .find_one(
                doc! { "server_id": id.to_string() },
                Some(
                    FindOneOptions::builder()
                        .hint(SERVER_ID_HINT.clone())
                        .build(),
                ),
            )
            .await?
        {
            return Ok(server);
        }

        let server = Self {
            server_id: id,
            admin_roles: Vec::new(),
            refrole: None,
        };

        servers.insert_one(&server, None).await?;

        Ok(server)
    }

    pub async fn set_refrole(&mut self, ctx: Context<'_>, role: RoleId) -> ClassResult<()> {
        if !ctx.guild().ok_or(ClassError::NoServer)?.roles.contains_key(&role) {
            return Err(ClassError::InvalidRole);
        }

        let new = Self {
            server_id: self.server_id,
            admin_roles: self.admin_roles.clone(),
            refrole: Some(role),
        };

        Self::get_collection().await.find_one_and_replace(
            doc! { "server_id": self.server_id.to_string() },
            &new,
            Some(FindOneAndReplaceOptions::builder()
                .hint(SERVER_ID_HINT.clone())
                .build()
            ),
        ).await?.ok_or(ClassError::NoServer)?;

        *self = new;

        Ok(())
    }

    async fn get_collection() -> Collection<Self> {
        static SERVERS: OnceCell<Collection<Server>> = OnceCell::const_new();

        SERVERS
            .get_or_init(|| async {
                get_conn()
                    .await
                    .database(&ENV.mongodb_name)
                    .collection("servers")
            })
            .await
            .clone()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct Class {
    server_id: GuildId,
    pub(crate) name: String,
    pub(crate) short_name: String,
    pub(crate) role: RoleId,
    pub(crate) category: ChannelId,
    pub(crate) text_channels: Vec<ChannelId>,
    pub(crate) voice_channels: Vec<ChannelId>,
}

impl Class {
    pub(crate) async fn list(server_id: GuildId) -> ClassResult<Vec<Class>> {
        Ok(
            Self::get_collection().await
                .find(
                    doc! { "server_id": server_id.to_string() },
                    Some(
                        FindOptions::builder()
                            .hint(SERVER_ID_HINT.clone())
                            .build(),
                    ),
                )
                .await?
                .try_collect::<Vec<_>>()
                .await?
        )
    }

    pub(crate) async fn create(ctx: Context<'_>, name: &str) -> ClassResult<Class> {
        let name = name.trim();

        let server = Server::get_or_create(ctx.guild_id().ok_or(ClassError::NoServer)?).await?;

        // Verify the server has a refrole set
        if server.refrole.is_none() {
            return Err(ClassError::NoRefrole);
        }
        // Verify the class does not already exist
        if Self::class_exists(server.server_id, name).await? {
            return Err(ClassError::ClassExists);
        }

        let guild = ctx.guild().ok_or(ClassError::NoServer)?;

        // Verify the role does not already exist
        if guild
            .roles
            .iter()
            .any(|(_, r)| r.name.to_lowercase() == name.to_lowercase())
        {
            return Err(ClassError::RoleExists);
        }
        // Verify the category does not already exist
        if guild.channels.iter().any(|(_, c)| {
            matches!(
                c, Channel::Category(cat)
                if cat.name.to_lowercase() == name.to_lowercase()
            )
        }) {
            return Err(ClassError::CategoryExists);
        }

        let http = ctx.discord().http();

        let position = guild
            .roles
            .get(&server.refrole.ok_or(ClassError::NoRefrole)?)
            .ok_or(ClassError::InvalidRefrole)?
            .position as u8;

        // Create the class role under the server refrole
        let role = guild
            .create_role(http, |r| r.name(name).mentionable(true).position(position))
            .await?;

        // Create the class category
        let category = guild
            .create_channel(http, |c| {
                c.name(name).kind(ChannelType::Category).permissions(vec![
                    PermissionOverwrite {
                        allow: Permissions::empty(),
                        deny: Permissions::VIEW_CHANNEL,
                        kind: PermissionOverwriteType::Role(guild.id.0.into()),
                    },
                    PermissionOverwrite {
                        allow: Permissions::VIEW_CHANNEL,
                        deny: Permissions::empty(),
                        kind: PermissionOverwriteType::Role(role.id),
                    },
                ])
            })
            .await?;

        // Create the class channels
        let short_name = name.split_whitespace().collect::<String>().to_lowercase();
        let general_channel = guild.create_channel(http, |c| {
            c.name(format!("general—〈{}〉", short_name))
                .kind(ChannelType::Text)
                .category(category.id)
        });
        let homework_help_channel = guild.create_channel(http, |c| {
            c.name(format!("homework-help—〈{}〉", short_name))
                .kind(ChannelType::Text)
                .category(category.id)
        });
        let resources_channel = guild.create_channel(http, |c| {
            c.name(format!("resources—〈{}〉", short_name))
                .kind(ChannelType::Text)
                .category(category.id)
        });
        let voice_channel = guild.create_channel(http, |c| {
            c.name(format!("General ({})", short_name))
                .kind(ChannelType::Voice)
                .category(category.id)
        });

        // Add the class to the database and return it
        Self {
            server_id: server.server_id,
            name: name.to_string(),
            short_name: short_name.clone(),
            role: role.id,
            category: category.id,
            text_channels: vec![
                general_channel.await?.id,
                homework_help_channel.await?.id,
                resources_channel.await?.id,
            ],
            voice_channels: vec![voice_channel.await?.id],
        }.add_to_db().await
    }

    pub(crate) async fn track(
        ctx: Context<'_>,
        name: Option<String>,
        role: Role,
        category: ChannelCategory,
        channels: &[GuildChannel],
    ) -> ClassResult<Class> {
        let guild = ctx.guild().ok_or(ClassError::NoServer)?;
        let server = Server::get_or_create(guild.id).await?;
        let name = name.as_ref().map(|s| s.trim()).unwrap_or(&role.name);

        // Verify the class does not already exist
        if Self::class_exists(guild.id, name).await? {
            return Err(ClassError::ClassExists);
        }

        // Verify another class is not already assigned to the same role
        if let Some(class) = Self::find_by_role(role.id).await? {
            return Err(ClassError::RoleInUse(class.name));
        }

        // Separate the text and voice channels and verify there are no other types of channels
        let mut text_channels = HashSet::new();
        let mut voice_channels = HashSet::new();
        for c in channels.iter().chain(
            guild.channels.iter()
                .filter_map(|(_, c)| if let Channel::Guild(gc) = c { Some(gc) } else { None })
                .filter(|c| c.parent_id.map(|id| id == category.id).unwrap_or(false))
        ) {
            match c.kind {
                ChannelType::Text => text_channels.insert(c.id),
                ChannelType::Voice => voice_channels.insert(c.id),
                _ => return Err(ClassError::InvalidChannelType(c.mention())),
            };
        }

        // Add the class to the database and return it
        Self {
            server_id: server.server_id,
            name: name.to_string(),
            short_name: name.split_whitespace().collect::<String>().to_lowercase(),
            role: role.id,
            category: category.id,
            text_channels: text_channels.into_iter().collect(),
            voice_channels: voice_channels.into_iter().collect(),
        }.add_to_db().await
    }

    pub(crate) async fn untrack(self) -> ClassResult<Option<String>> {
        let deleted_count = Self::get_collection().await
            .delete_many(
                doc! { "role": self.role.to_string() },
                DeleteOptions::builder()
                    .hint(ROLE_HINT.clone())
                    .build()
            ).await?.deleted_count;

        Ok(
            if deleted_count > 0 {
                Some(self.name)
            } else { None }
        )
    }

    pub(crate) async fn delete(self, ctx: Context<'_>) -> ClassResult<(Option<String>, Vec<ClassError>)> {
        let mut guild = ctx.guild().ok_or(ClassError::NoServer)?;
        let http = ctx.discord().http();

        let db_deleted = self.clone().untrack().await?.is_some();

        let mut failed = Vec::new();

        for c in self.text_channels.iter()
            .chain(self.voice_channels.iter())
            .chain(std::iter::once(&self.category))
        {
            if let Some(channel) = guild.channels.get(c) {
                if let Err(e) = channel.delete(http).await {
                    failed.push(ClassError::ApiError(e))
                }
            } else {
                failed.push(ClassError::InvalidChannel(c.mention()));
            }
        }

        if let Err(e) = futures::future::ready(
            guild.roles.get_mut(&self.role)
                .ok_or(ClassError::InvalidRole)
        )
            .and_then(|r| r.delete(http).map_err(ClassError::ApiError))
            .await
        {
            failed.push(e);
        }

        Ok((
            if db_deleted {
                Some(self.name)
            } else { None },
            failed,
        ))
    }

    async fn get_collection() -> Collection<Self> {
        static CLASSES: OnceCell<Collection<Class>> = OnceCell::const_new();

        CLASSES
            .get_or_init(|| async {
                get_conn()
                    .await
                    .database(&ENV.mongodb_name)
                    .collection("classes")
            })
            .await
            .clone()
    }

    async fn class_exists(server_id: GuildId, name: &str) -> ClassResult<bool> {
        Ok(
            Self::get_collection().await
                .find_one(
                    doc! { "server_id": server_id.to_string(), "name": name },
                    Some(
                        FindOneOptions::builder()
                            .hint(SERVER_ID_NAME_HINT.clone())
                            .build(),
                    ),
                )
                .await?
                .is_some()
        )
    }

    async fn add_to_db(self) -> ClassResult<Class> {
        Self::get_collection().await.insert_one(&self, None).await?;
        Ok(self)
    }

    pub(crate) async fn find_by_role(role: RoleId) -> ClassResult<Option<Class>> {
        Ok(
            Self::get_collection().await.find_one(
                doc! { "role": role.to_string() },
                Some(
                    FindOneOptions::builder()
                        .hint(ROLE_HINT.clone())
                        .build()
                )
            ).await?
        )
    }
}


