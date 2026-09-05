//! Verificação ao vivo da categoria Instagram com uma sessão real. Ignorado
//! por padrão; roda com um cookies.txt (Netscape) do instagram.com:
//!
//! ```text
//! OMNIGET_IG_COOKIES="$HOME/Library/Application Support/wtf.tonho.omniget/cookies/instagram.com/_default.txt" \
//!   cargo test -p omniget-core --test instagram_live -- --ignored --nocapture
//! ```
//!
//! Só faz leituras (perfil, posts, reels, stories, highlights, seguidores,
//! comentários, curtidas, hashtag). Nada de unfollow nem publicação.

use std::sync::atomic::AtomicBool;

use omniget_core::core::tools::instagram::{follow, media, profile, social, IgClient, Session};
use omniget_core::core::tools::noop_progress;

#[tokio::test]
#[ignore]
async fn ig_live_read_only() {
    let path = std::env::var("OMNIGET_IG_COOKIES").expect("OMNIGET_IG_COOKIES");
    let content = std::fs::read_to_string(&path).expect("cookies.txt");
    let session = Session::from_netscape(&content).expect("sessionid + ds_user_id");
    let client = IgClient::new(session).unwrap();
    let flag = AtomicBool::new(false);
    let p = noop_progress();

    let me = profile::whoami(&client).await.expect("whoami");
    println!("logado como @{} ({} seguidores)", me.username, me.follower_count);

    let target = std::env::var("OMNIGET_IG_TARGET").unwrap_or_else(|_| "instagram".into());
    let user = profile::resolve_user(&client, &target).await.expect("resolve_user");
    println!("@{} pk={} posts={} privado={}", user.username, user.pk, user.media_count, user.is_private);

    let posts = profile::user_posts(&client, &user.username, 15, &flag, &p, "t").await.expect("user_posts (graphql)");
    println!("posts: {} (primeiro {} · {} arquivos)", posts.len(), posts.first().map(|x| x.code.as_str()).unwrap_or("-"), posts.first().map(|x| x.files.len()).unwrap_or(0));
    assert!(posts.len() > 12, "paginação do GraphQL devia passar de 12");

    let reels = profile::user_reels(&client, &user.pk, 5, &flag, &p, "t").await.expect("user_reels");
    println!("reels: {}", reels.len());

    let tagged = profile::user_tagged(&client, &user.pk, 5, &flag, &p, "t").await.expect("user_tagged");
    println!("marcados: {}", tagged.len());

    let hls = profile::highlights(&client, &user.pk).await.expect("highlights");
    println!("highlights: {}", hls.len());
    if let Some(h) = hls.first() {
        let items = profile::highlight_items(&client, &h.id).await.expect("highlight_items");
        println!("  '{}' com {} itens", h.title, items.len());
    }
    let stories = profile::stories_of(&client, &user.pk).await.expect("stories_of");
    println!("stories ativos: {}", stories.len());

    if let Some(post) = posts.first() {
        let item = media::post_info(&client, &post.code).await.expect("post_info");
        println!("post_info ok: {} likes, {} comentarios", item.like_count, item.comment_count);
        let comments = social::comments(&client, &item.pk, 30, &flag, &p, "t").await.expect("comments");
        let (likes, likers) = social::likers(&client, &item.pk).await.expect("likers");
        println!("comentarios: {} · likers: {} de {}", comments.len(), likers.len(), likes);
    }

    let following = follow::following(&client, &me.pk, 60, &flag, &p, "t").await.expect("following");
    let followers = follow::followers(&client, &me.pk, 60, &flag, &p, "t").await.expect("followers");
    let a = follow::analyze(followers, following, &Default::default());
    println!("seguidores {} · seguindo {} · nao seguem de volta (amostra) {}", a.followers_count, a.following_count, a.not_following_back.len());

    let tag = social::tag_info(&client, "sunset").await.expect("tag_info");
    println!("#sunset: {} posts", tag.media_count);
    let tag_media = social::tag_media(&client, "sunset", "recent", 10, &flag, &p, "t").await.unwrap_or_default();
    println!("#sunset recentes: {}", tag_media.len());
}
