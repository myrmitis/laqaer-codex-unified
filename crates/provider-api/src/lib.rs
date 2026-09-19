use async_trait::async_trait;
use codex_unified_core::{
    Provider, ProviderCapabilities, ProviderError, ProviderEventStream, ProviderResolver,
};
use codex_unified_protocol::{CanonicalEvent, TurnEnvelope};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiProtocol {
    OpenAiResponses,
    OpenAiCompatibleResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiRoute {
    pub provider_id: String,
    pub model_prefix: String,
    pub upstream_base_url: String,
    pub protocol: ApiProtocol,
    pub credential_ref: String,
}

impl ApiRoute {
    pub fn matches(&self, model: &str) -> bool {
        model.starts_with(&self.model_prefix)
    }

    pub fn upstream_model<'a>(&self, model: &'a str) -> Option<&'a str> {
        model.strip_prefix(&self.model_prefix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResolution<'a> {
    Matched {
        route: &'a ApiRoute,
        upstream_model: &'a str,
    },
    NoMatch,
    Ambiguous,
}

#[derive(Debug, Clone, Default)]
pub struct ApiRouteTable {
    routes: Vec<ApiRoute>,
}

impl ApiRouteTable {
    pub fn new(routes: Vec<ApiRoute>) -> Self {
        Self { routes }
    }

    pub fn resolve<'a>(&'a self, model: &'a str) -> RouteResolution<'a> {
        let mut matches = self.routes.iter().filter_map(|route| {
            route
                .upstream_model(model)
                .map(|upstream| (route, upstream))
        });

        let Some((route, upstream_model)) = matches.next() else {
            return RouteResolution::NoMatch;
        };

        if matches.next().is_some() {
            return RouteResolution::Ambiguous;
        }

        RouteResolution::Matched {
            route,
            upstream_model,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiProvider {
    pub route: ApiRoute,
}

#[async_trait]
impl Provider for ApiProvider {
    fn id(&self) -> &'static str {
        "api"
    }

    async fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tools: true,
            images: true,
            continuation: true,
        }
    }

    async fn execute(&self, turn: TurnEnvelope) -> Result<ProviderEventStream, ProviderError> {
        // Live HTTP forwarding lands in the next slice. This stub exercises the
        // canonical provider stream without inventing provider-specific transforms.
        let response_id = format!("api-stub-{}", turn.identity.turn_id);
        Ok(Box::pin(stream::iter(vec![
            Ok(CanonicalEvent::ResponseCreated {
                response_id: response_id.clone(),
            }),
            Ok(CanonicalEvent::ResponseCompleted { response_id }),
        ])))
    }
}

#[derive(Debug, Clone)]
pub struct ApiProviderResolver {
    routes: ApiRouteTable,
}

impl ApiProviderResolver {
    pub fn new(routes: ApiRouteTable) -> Self {
        Self { routes }
    }
}

impl ProviderResolver for ApiProviderResolver {
    fn resolve(&self, model: &str) -> Option<Arc<dyn Provider>> {
        match self.routes.resolve(model) {
            RouteResolution::Matched { route, .. } => Some(Arc::new(ApiProvider {
                route: route.clone(),
            })),
            RouteResolution::NoMatch | RouteResolution::Ambiguous => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openrouter() -> ApiRoute {
        ApiRoute {
            provider_id: "openrouter".into(),
            model_prefix: "openrouter/".into(),
            upstream_base_url: "https://openrouter.example/v1".into(),
            protocol: ApiProtocol::OpenAiCompatibleResponses,
            credential_ref: "keychain://openrouter".into(),
        }
    }

    fn grok() -> ApiRoute {
        ApiRoute {
            provider_id: "xai".into(),
            model_prefix: "grok-oauth/".into(),
            upstream_base_url: "https://xai.example/v1".into(),
            protocol: ApiProtocol::OpenAiResponses,
            credential_ref: "oauth://xai".into(),
        }
    }

    #[test]
    fn resolves_openrouter_without_touching_grok() {
        let table = ApiRouteTable::new(vec![openrouter(), grok()]);
        let RouteResolution::Matched {
            route,
            upstream_model,
        } = table.resolve("openrouter/meta/llama")
        else {
            panic!("expected one route");
        };

        assert_eq!(route.provider_id, "openrouter");
        assert_eq!(upstream_model, "meta/llama");
    }

    #[test]
    fn resolves_grok_as_independent_route() {
        let table = ApiRouteTable::new(vec![openrouter(), grok()]);
        let RouteResolution::Matched {
            route,
            upstream_model,
        } = table.resolve("grok-oauth/grok-4.6")
        else {
            panic!("expected one route");
        };

        assert_eq!(route.provider_id, "xai");
        assert_eq!(upstream_model, "grok-4.6");
    }

    #[test]
    fn ambiguous_prefixes_fail_closed() {
        let mut duplicate = openrouter();
        duplicate.provider_id = "other".into();
        let table = ApiRouteTable::new(vec![openrouter(), duplicate]);

        assert_eq!(
            table.resolve("openrouter/model"),
            RouteResolution::Ambiguous
        );
    }

    #[test]
    fn resolver_refuses_ambiguous_route() {
        let mut duplicate = openrouter();
        duplicate.provider_id = "other".into();
        let resolver = ApiProviderResolver::new(ApiRouteTable::new(vec![openrouter(), duplicate]));
        assert!(resolver.resolve("openrouter/model").is_none());
    }
}
