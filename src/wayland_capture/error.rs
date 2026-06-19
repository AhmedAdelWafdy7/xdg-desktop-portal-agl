use thiserror::Error;

use crate::shell::SurfaceRole;

#[derive(Error, Debug)]
pub enum ShellError {
    #[error("Surface role has already been assigned and cannot be changed")]
    RoleAlreadyAssigned,

    #[error("Role {0:?} is not compatible with the current shell backend")]
    IncompatibleRole(SurfaceRole),

    #[error("Shell Surface is not initialized properly")]
    UninitializedSurface,
}
