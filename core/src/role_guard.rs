//! Role guard for administrative API operations (API-04).
//!
//! Roles come from the verified JWT claims (via [`AuthenticatedUser`]); a role
//! in a request body is never consulted. Denials are always `403 Forbidden`.
//!
//! | Permission      | Roles allowed                | Typical operations                          |
//! |-----------------|------------------------------|---------------------------------------------|
//! | `ReadOnly`      | any authenticated role       | list/get own vaults and reports             |
//! | `Manager`       | `manager`, `admin`           | create/delete vaults, issue scoped tokens   |
//! | `ProtocolAdmin` | `admin` (or allow-listed)    | restore vaults, list deleted vaults, manager approval, log levels, emergency pause |

use crate::auth::AuthenticatedUser;
use crate::errors::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ReadOnly,
    Manager,
    ProtocolAdmin,
}

/// Fail with `403 Forbidden` unless `user` holds `permission`.
pub fn require(user: &AuthenticatedUser, permission: Permission) -> Result<(), AppError> {
    let (allowed, needed) = match permission {
        Permission::ReadOnly => (true, "Authenticated"),
        Permission::Manager => (user.can_manage_vaults(), "Manager"),
        Permission::ProtocolAdmin => (user.is_admin(), "Administrator"),
    };
    if allowed {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "{needed} role is required; role '{}' is not permitted",
            user.role
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;

    #[test]
    fn role_matrix() {
        // (role, read_only, manager, protocol_admin)
        let matrix = [
            (Role::Viewer, true, false, false),
            (Role::Operator, true, false, false),
            (Role::Manager, true, true, false),
            (Role::Admin, true, true, true),
        ];
        for (role, read, manage, admin) in matrix {
            let user = AuthenticatedUser::new("GROLEGUARDTEST".into(), role, vec![]);
            assert_eq!(require(&user, Permission::ReadOnly).is_ok(), read, "{role}");
            assert_eq!(require(&user, Permission::Manager).is_ok(), manage, "{role}");
            assert_eq!(require(&user, Permission::ProtocolAdmin).is_ok(), admin, "{role}");
        }
    }

    #[test]
    fn denial_is_forbidden() {
        let user = AuthenticatedUser::new("GROLEGUARDTEST".into(), Role::Viewer, vec![]);
        assert!(matches!(
            require(&user, Permission::ProtocolAdmin),
            Err(AppError::Forbidden(_))
        ));
    }
}
