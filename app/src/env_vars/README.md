# Environment Variables Documentation

This document provides information about our "Environment Variables" feature. Internally, we refer to these objects as `EnvVarCollection`s (EVCs). Views bound to this object are often referred to by the string above, whereas functions and variables are usually named `env_var_collection`.

This documentation is up-to-date as of 6/26/2024. All referenced files are present in this directory unless specified otherwise.

## Core Data Models

The core data model for EVCs is defined in `mod.rs`. The motivations behind our data model are detailed in the above documents, with the v1 tech doc being the most relevant.

## Cloud Infrastructure

Context: EVCs are built on GenericStringObjects (GSOs). Consequently, there isn't much unique server-side infrastructure dedicated to EVCs — we added a variant to the `Format` enum on the server side and did the same on the client (`JsonObjectType::EnvVarCollection`), and a small DB migration to support the type.

We defined `CloudEnvVarCollection` in `mod.rs`, which implements the `GenericCloudObjectType` trait. This is a mostly boilerplate implementation specifying properties such as EVCs should render in Warp Drive, be linkable/exportable, etc.

The implementation of EVCs as a Warp Drive object is in `app/src/drive/items/env_var_collection.rs`, where code for the Warp Drive preview and click action is located.

Code relevant to edit collisions and fetching EVCs from the server is in `app/src/server/server_api.rs` and `app/src/server/cloud_objects/update_manager.rs`. We aimed to maintain a similar liveness property to workflows, meaning a concurrent edit made by another user requires one to check out the other's edit before committing their own.

## Client Side

### Panes

EVCs, like most objects in Warp, are children of a pane. Our implementation is defined in `app/src/pane_group/pane/env_var_collection_pane.rs`, which is essentially identical to other pane implementations. The `EnvVarCollectionPane` is closely coupled with the `EnvVarCollectionManager`, defined in `manager.rs`. The manager is responsible for creating, destroying, and registering all EVC panes, whereas the pane itself contains the EVC view.

### Core UI

We'll describe our core UI components by line-by-lining each file in the view directory, ordered by importance.

- `env_var_collection.rs` — Contains the core functions and implementation of the `EnvVarCollectionView`. Functions like "open_new_env_var_collection" and "load" (which loads an existing EVC or reloads an open EVC after a collision) are documented with descriptions of their relevance.
- `secrets.rs` — separate section below as it's a crucial flow
- `command_dialog`
    - `command_dialog_view.rs` — Defines the view for the command dialog.
    - `mod.rs` — Contains functionality related to the command dialog i.e. (listening to events from the dialog)
- `unsaved_changes_dialog.rs` — Contains code related to the dialog presented when a user tries to close the pane without saving changes.
- `menus.rs` — Defines menu-related code for EVCs. This includes secret menus (linked to the key icon or a rendered secret/command) and pane-bound menus (overflow menu with object-specific actions and the context menu with split pane actions, triggered on right-click).
- `editors.rs` — Defines code for initializing editors, handling their events (such as tab navigation), and rendering the "metadata" section.
- `fixed_view_components.rs` — Contains render functions for components like the trash overflow banner or the save button in the footer.
- `active_env_var_collection_data.rs` — Tracks the currently open EVC, including the current revision and saving status.

### Secrets

Secret initialization can be best described by examining the full flow:

1. The user clicks on a menu linked to a row (key icon or rendered secret/command), dispatching a `DisplaySecretMenu(VariableRowIndex)` action.
2. The action is handled, storing the `VariableRowIndex` in the `pending_variable_row_index` state variable.
3. The user selects a menu item (e.g., 1password), triggering a `SelectSecretManager` action, which resolves to the `fetch_secret` function.
4. In `fetch_secret`, the following occurs:
    1. Data about the user's local shell is retrieved to run the command which fetches all the user's secrets
    2. On a background thread, the `verify_installed_and_fetch_secrets` function in `app/src/external_secrets/mod.rs` is executed. This function checks if the selected secret manager is installed and tries to fetch secrets using the aforementioned local_shell module (well documented). If either operation fails, `fetch_secret` displays an error toast.
5. Assuming secrets are successfully fetched, they are sent to the searchable secrets dialog (located in `app/src/search/external_secrets`), which propagates an event back to the EVC view to indicate the dialog should be opened.
6. The user selects a secret, propagating an event to the EVC view, which stores the secret in the value field of the `VariableEditorRow` pointed to by `pending_variable_row_index` and closes the dialog.

### Other

- Code for the EVC portion of the workflow card (parameterized workflows) is defined in `app/src/workflows/info_box.rs`.
- Code related to command palette and search functionality is in their respective directories located in `app/src/search`.
- Code for the EVC block appended to the blocklist prior to invocation is in `env_var_collection_block.rs`. Commands that set/initialize variables are established in `mod.rs`. The codepath for invoking an EVC is in `invoke_environment_variables` of `app/src/terminal/view.rs`.
