//! The RFC 2228 Data Channel Protection Level (`PROT`) command.

use crate::{
    auth::UserDetail,
    server::controlchan::{
        error::ControlChanError,
        handler::{CommandContext, CommandHandler},
        Reply, ReplyCode,
    },
    storage::{Metadata, StorageBackend},
};
use async_trait::async_trait;

// The parameter that can be given to the `PROT` command.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ProtParam {
    // 'C' - Clear - neither Integrity nor Privacy
    Clear,
    // 'S' - Safe - Integrity without Privacy
    Safe,
    // 'E' - Confidential - Privacy without Integrity
    Confidential,
    // 'P' - Private - Integrity and Privacy
    Private,
}

#[derive(Debug)]
pub struct Prot {
    param: ProtParam,
}

impl Prot {
    pub fn new(param: ProtParam) -> Self {
        Prot { param }
    }
}

#[async_trait]
impl<Storage, User> CommandHandler<Storage, User> for Prot
where
    User: UserDetail,
    Storage: StorageBackend<User> + 'static,
    Storage::Metadata: 'static + Metadata,
{
    #[tracing_attributes::instrument]
    async fn handle(&self, _args: CommandContext<Storage, User>) -> Result<Reply, ControlChanError> {
        let tls = {
            #[cfg(feature = "tls")]
            {
                _args.tls_configured
            }
            #[cfg(not(feature = "tls"))]
            {
                true
            }
        };
        match (tls, self.param.clone()) {
            (true, ProtParam::Clear) => {
                #[cfg(feature = "tls")]
                {
                    let mut session = _args.session.lock().await;
                    session.data_tls = false;
                }
                Ok(Reply::new(ReplyCode::CommandOkay, "PROT OK. Switching data channel to plaintext"))
            }
            (true, ProtParam::Private) => {
                #[cfg(not(feature = "tls"))]
                {
                    unimplemented!("unimplemented tls!")
                }
                #[cfg(feature = "tls")]
                {
                    let mut session = _args.session.lock().await;
                    session.data_tls = true;
                    Ok(Reply::new(ReplyCode::CommandOkay, "PROT OK. Securing data channel"))
                }
            }
            (true, _) => Ok(Reply::new(ReplyCode::CommandNotImplementedForParameter, "PROT S/E not implemented")),
            (false, _) => Ok(Reply::new(ReplyCode::CommandNotImplemented, "TLS/SSL not configured")),
        }
    }
}
