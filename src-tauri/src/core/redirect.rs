use anyhow::anyhow;

pub async fn resolve_redirect(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .send()
        .await?;

    let final_url = response.url().to_string();

    if final_url == url {
        return Err(anyhow!("Nenhum redirect encontrado para {}", url));
    }

    Ok(final_url)
}
