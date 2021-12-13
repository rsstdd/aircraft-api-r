use aircraft_db_schema::newtypes::{CommunityId, PersonId};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Search {
  pub q: String,
  pub community_id: Option<CommunityId>,
  pub community_name: Option<String>,
  pub creator_id: Option<PersonId>,
  pub type_: Option<String>,
  pub sort: Option<String>,
  pub listing_type: Option<String>,
  pub page: Option<i64>,
  pub limit: Option<i64>,
  pub auth: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SearchResponse {
  pub type_: String,
  pub comments: Vec<CommentView>,
  pub posts: Vec<PostView>,
  pub communities: Vec<CommunityView>,
  pub users: Vec<PersonViewSafe>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ResolveObject {
  pub q: String,
  pub auth: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct ResolveObjectResponse {
  pub comment: Option<CommentView>,
  pub post: Option<PostView>,
  pub community: Option<CommunityView>,
  pub person: Option<PersonViewSafe>,
}

#[derive(Serialize, Deserialize)]
pub struct GetModlog {
  pub mod_person_id: Option<PersonId>,
  pub community_id: Option<CommunityId>,
  pub page: Option<i64>,
  pub limit: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct GetModlogResponse {
  pub removed_posts: Vec<ModRemovePostView>,
  pub locked_posts: Vec<ModLockPostView>,
  pub stickied_posts: Vec<ModStickyPostView>,
  pub removed_comments: Vec<ModRemoveCommentView>,
  pub removed_communities: Vec<ModRemoveCommunityView>,
  pub banned_from_community: Vec<ModBanFromCommunityView>,
  pub banned: Vec<ModBanView>,
  pub added_to_community: Vec<ModAddCommunityView>,
  pub transferred_to_community: Vec<ModTransferCommunityView>,
  pub added: Vec<ModAddView>,
}

#[derive(Serialize, Deserialize)]
pub struct CreateSite {
  pub name: String,
  pub sidebar: Option<String>,
  pub description: Option<String>,
  pub icon: Option<String>,
  pub banner: Option<String>,
  pub enable_downvotes: Option<bool>,
  pub open_registration: Option<bool>,
  pub enable_nsfw: Option<bool>,
  pub community_creation_admin_only: Option<bool>,
  pub auth: String,
}

#[derive(Serialize, Deserialize)]
pub struct EditSite {
  pub name: Option<String>,
  pub sidebar: Option<String>,
  pub description: Option<String>,
  pub icon: Option<String>,
  pub banner: Option<String>,
  pub enable_downvotes: Option<bool>,
  pub open_registration: Option<bool>,
  pub enable_nsfw: Option<bool>,
  pub community_creation_admin_only: Option<bool>,
  pub auth: String,
}

#[derive(Serialize, Deserialize)]
pub struct GetSite {
  pub auth: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SiteResponse {
  pub site_view: SiteView,
}

#[derive(Serialize, Deserialize)]
pub struct GetSiteResponse {
  pub site_view: Option<SiteView>, // Because the site might not be set up yet
  pub admins: Vec<PersonViewSafe>,
  pub banned: Vec<PersonViewSafe>,
  pub online: usize,
  pub version: String,
  pub my_user: Option<MyUserInfo>,
  pub federated_instances: Option<FederatedInstances>, // Federation may be disabled
}

#[derive(Serialize, Deserialize)]
pub struct MyUserInfo {
  pub local_user_view: LocalUserSettingsView,
  pub follows: Vec<CommunityFollowerView>,
  pub moderates: Vec<CommunityModeratorView>,
  pub community_blocks: Vec<CommunityBlockView>,
  pub person_blocks: Vec<PersonBlockView>,
}

#[derive(Serialize, Deserialize)]
pub struct TransferSite {
  pub person_id: PersonId,
  pub auth: String,
}

#[derive(Serialize, Deserialize)]
pub struct GetSiteConfig {
  pub auth: String,
}

#[derive(Serialize, Deserialize)]
pub struct GetSiteConfigResponse {
  pub config_hjson: String,
}

#[derive(Serialize, Deserialize)]
pub struct SaveSiteConfig {
  pub config_hjson: String,
  pub auth: String,
}

#[derive(Serialize, Deserialize)]
pub struct FederatedInstances {
  pub linked: Vec<String>,
  pub allowed: Option<Vec<String>>,
  pub blocked: Option<Vec<String>>,
}
