//! Web search through the gateway; a search's credits count toward the session that ran it.

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;

use crate::credits::Credits;
use crate::gateway::Gateway;
use crate::tool::websearch::{SearchResult, WebSearch};

pub struct GatewaySearch {
    gateway: Arc<Gateway>,
    credits: Arc<Credits>,
}

impl GatewaySearch {
    pub fn new(gateway: Arc<Gateway>, credits: Arc<Credits>) -> Self {
        GatewaySearch { gateway, credits }
    }
}

impl WebSearch for GatewaySearch {
    fn search<'a>(
        &'a self,
        session_id: &'a str,
        query: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SearchResult>, String>> {
        async move {
            let (results, credits) = self.gateway.search(query).await?;
            if credits > 0 {
                self.credits.record(session_id, Some(credits), None);
            }
            Ok(results)
        }
        .boxed()
    }
}
