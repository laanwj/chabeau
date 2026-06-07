use super::usage_status;
use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::App;
use crate::core::session_store::{list_sessions, save_session, SessionSummary};
use crate::ui::picker::PickerItem;

const USAGE_SAVE: &str = "Usage: /save [name]";
const USAGE_LOAD: &str = "Usage: /load [session-id]";

pub(crate) fn handle_save(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    let name = match invocation.args_len() {
        0 => generate_session_name(&app.ui.messages),
        1 => invocation.arg(0).unwrap_or("Untitled").to_string(),
        _ => return usage_status(app, USAGE_SAVE),
    };

    let messages = app.ui.messages.iter().cloned().collect::<Vec<_>>();
    let result = save_session(
        &app.session.session_id,
        &name,
        &app.session.provider_name,
        &app.session.model,
        &app.session.base_url,
        app.session.get_character().cloned(),
        app.persona_manager.get_active_persona().cloned(),
        app.preset_manager.get_active_preset().cloned(),
        &messages,
        &app.session.tool_pipeline.tool_results,
        &app.session.tool_pipeline.tool_payload_history,
        &app.session.mcp_init,
        &app.session.refine_instructions,
        &app.session.refine_prefix,
        app.ui.markdown_enabled,
        app.ui.syntax_enabled,
    );

    match result {
        Ok(()) => {
            let session_id = app.session.session_id.clone();
            app.conversation()
                .set_status(format!("Session saved: {} ({})", name, session_id));
            CommandResult::ContinueWithTranscriptFocus
        }
        Err(e) => {
            app.conversation().set_status(format!("Save error: {}", e));
            CommandResult::ContinueWithTranscriptFocus
        }
    }
}

pub(crate) fn handle_load(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    match invocation.args_len() {
        0 => match list_sessions() {
            Ok(sessions) => {
                if sessions.is_empty() {
                    app.conversation().set_status("No saved sessions found.");
                    return CommandResult::ContinueWithTranscriptFocus;
                }
                show_session_picker(app, sessions);
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Session list error: {}", e));
                CommandResult::ContinueWithTranscriptFocus
            }
        },
        1 => {
            let Some(id) = invocation.arg(0) else {
                return usage_status(app, USAGE_LOAD);
            };
            match do_load_session(app, id) {
                Ok(()) => CommandResult::ContinueWithTranscriptFocus,
                Err(e) => {
                    app.conversation().set_status(format!("Load error: {}", e));
                    CommandResult::ContinueWithTranscriptFocus
                }
            }
        }
        _ => usage_status(app, USAGE_LOAD),
    }
}

pub(crate) fn handle_sessions(app: &mut App, _invocation: CommandInvocation<'_>) -> CommandResult {
    match list_sessions() {
        Ok(sessions) => {
            if sessions.is_empty() {
                app.conversation().set_status("No saved sessions found.");
                return CommandResult::ContinueWithTranscriptFocus;
            }
            show_session_picker(app, sessions);
            CommandResult::Continue
        }
        Err(e) => {
            app.conversation()
                .set_status(format!("Session list error: {}", e));
            CommandResult::ContinueWithTranscriptFocus
        }
    }
}

fn generate_session_name(
    messages: &std::collections::VecDeque<crate::core::message::Message>,
) -> String {
    for msg in messages {
        if msg.is_user() && !msg.content.is_empty() {
            let first_line = msg.content.lines().next().unwrap_or("");
            let truncated = if first_line.chars().count() > 50 {
                first_line.chars().take(47).collect::<String>() + "..."
            } else {
                first_line.to_string()
            };
            return truncated;
        }
    }
    "Untitled session".to_string()
}

pub fn do_load_session(app: &mut App, id: &str) -> Result<(), String> {
    let snapshot = crate::core::session_store::load_session(id).map_err(|e| e.to_string())?;

    app.session.session_id = snapshot.id.clone();
    app.ui.messages = snapshot.messages.into_iter().collect();
    app.ui.invalidate_prewrap_cache();

    app.session.tool_pipeline.tool_results = snapshot.tool_results;
    app.session.tool_pipeline.tool_payload_history = snapshot
        .tool_payloads
        .into_iter()
        .map(|tp| crate::core::app::session::ToolPayloadHistoryEntry {
            server_id: tp.server_id,
            tool_call_id: tp.tool_call_id,
            assistant_message: tp.assistant_message,
            tool_message: tp.tool_message,
            assistant_message_index: tp.assistant_message_index,
        })
        .collect();

    app.session.mcp_init = crate::core::app::session::McpInitState {
        in_progress: false,
        complete: snapshot.mcp_init_complete,
        deferred_message: None,
    };

    app.session.refine_instructions = snapshot.refine_instructions;
    app.session.refine_prefix = snapshot.refine_prefix;

    app.ui.markdown_enabled = snapshot.markdown_enabled;
    app.ui.syntax_enabled = snapshot.syntax_enabled;

    app.session.provider_name = snapshot.provider;
    app.session.model = snapshot.model;
    app.session.base_url = snapshot.base_url;

    // Restore character, persona, preset from stored data
    if let Some(character) = snapshot.character {
        app.session.set_character(character);
    }
    if let Some(persona) = snapshot.persona {
        app.persona_manager.set_active_persona(&persona.id).ok();
    }
    if let Some(preset) = snapshot.preset {
        app.preset_manager.set_active_preset(&preset.id).ok();
    }

    app.conversation()
        .set_status(format!("Loaded session: {}", snapshot.name));

    Ok(())
}

fn show_session_picker(app: &mut App, sessions: Vec<SessionSummary>) {
    let items: Vec<PickerItem> = sessions
        .iter()
        .map(|s| {
            let provider = &s.provider;
            let model = &s.model;
            let char_label = s.character.as_deref().unwrap_or("-");
            let meta = format!(
                "{} | {} | {} | {} msgs | {}",
                provider,
                model,
                char_label,
                s.message_count,
                s.modified_at.format("%Y-%m-%d %H:%M")
            );
            PickerItem {
                id: s.id.clone(),
                label: s.name.clone(),
                metadata: Some(meta),
                inspect_metadata: None,
                sort_key: Some(s.modified_at.to_string()),
            }
        })
        .collect();

    app.picker.open_session_picker(sessions, items);
}
