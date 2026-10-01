use serde_json::Value;

#[derive(Debug, Clone)]
pub struct NodeCtx {
    pub node_id: String,
    pub workspace_id: String,
    pub floor_id: String,
    pub scopes: Vec<String>,
}

pub type RpcResult = Option<Result<Value, String>>;

pub async fn dispatch(ctx: &NodeCtx, method: &str, args: Value) -> Result<Value, String> {
    if let Some(r) = super::cords::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::roles::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::chat::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::portal::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::floors::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::partituras::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::routines::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::maestro::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::companion::rpc(ctx, method, args.clone()).await {
        return r;
    }
    if let Some(r) = super::remote::rpc(ctx, method, args.clone()).await {
        return r;
    }
    Err(format!("unknown canvas method `{method}`"))
}
