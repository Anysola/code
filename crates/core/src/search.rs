//! Web search through the gateway. What a search takes of the usage windows shows in the next
//! usage event or `/limits` poll.

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;

use crate::gateway::Gateway;
use crate::tool::websearch::{SearchResult, WebSearch};

pub struct GatewaySearch {
    gateway: Arc<Gateway>,
}

impl GatewaySearch {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        GatewaySearch { gateway }
    }
}

impl WebSearch for GatewaySearch {
    fn search<'a>(
        &'a self,
        _session_id: &'a str,
        query: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, String>> {
        async move { self.gateway.search(query).await }.boxed()
    }
}
