use crate::external_secrets::ExternalSecret;
use crate::search::mixer::SearchMixer;

pub type ExternalSecretSearchMixer = SearchMixer<ExternalSecretSearchItemAction>;

#[derive(Clone, Debug)]
pub enum ExternalSecretSearchItemAction {
    AcceptSecret(ExternalSecret),
}
