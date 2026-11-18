//! The only code that writes the window, and the binding of its callbacks to commands.

use slint::{ModelRc, SharedString, VecModel};
use yantrik_app::app::Changed;
use yantrik_app::cx::Cx;
use yantrik_app::rail::{self, RailView as _};

use crate::model::{Command, Model, Verb, PART_AI, PART_LISTING, PART_LOGS, PART_NOTICE, PART_TAB};
use crate::runtime::plain_image_name;
use crate::{ContainerData, ContainerManagerApp, Containers, ImageData, VolumeData};

/// Each control, to the command it runs.
///
/// Each button is the same command the matching action raises, so there is nothing on the
/// person's path that the mind's path skips. The tab bar switches the list itself
/// (container_manager.slint); `ct-tab` tells the model, which `describe` reads. No
/// `agent-context-activated`: the rail's context rows are drawn as plain rows, not buttons
/// (`context-opens` stays false). No input field is read back: the screen clears each once its
/// command is raised.
pub fn wire(cx: &Cx<Containers>) {
    let ui = cx.ui();
    ui.on_ct_tab(cx.cmd1(|tab: i32| Command::ShowTab(tab.clamp(0, 2) as usize)));
    ui.on_ct_refresh(cx.cmd(|| Command::Refresh));
    ui.on_ct_start(cx.cmd1(|id: SharedString| row(Verb::Start, id)));
    ui.on_ct_stop(cx.cmd1(|id: SharedString| row(Verb::Stop, id)));
    ui.on_ct_restart(cx.cmd1(|id: SharedString| row(Verb::Restart, id)));
    ui.on_ct_remove(cx.cmd1(|id: SharedString| row(Verb::Remove, id)));
    ui.on_ct_logs(cx.cmd1(|id: SharedString| row(Verb::Logs, id)));
    ui.on_ct_run(cx.cmd1(|image: SharedString| Command::RunImage(image.to_string())));
    ui.on_ct_pull(cx.cmd1(|image: SharedString| Command::PullImage(image.to_string())));
    ui.on_ct_remove_image(cx.cmd1(|id: SharedString| Command::RemoveImage(id.to_string())));
    ui.on_ct_create_volume(cx.cmd1(|name: SharedString| Command::CreateVolume(name.to_string())));
    ui.on_ct_remove_volume(cx.cmd1(|name: SharedString| Command::RemoveVolume(name.to_string())));
    ui.on_logs_closed(cx.cmd(|| Command::LogsClosed));
    ui.on_ai_summarize_logs(cx.cmd(|| Command::Summarize));
    ui.on_ai_dismiss(cx.cmd(|| Command::DismissAi));
    drop(ui);
    rail::wire(cx);
}

fn row(verb: Verb, id: SharedString) -> Command {
    Command::Row { verb, id: id.to_string() }
}

pub fn render(ui: &ContainerManagerApp, m: &Model, changed: &Changed) {
    if changed.has(PART_LISTING) {
        listing(ui, m);
    }
    if changed.has(PART_NOTICE) {
        ui.set_notice(m.notice.as_str().into());
    }
    if changed.has(PART_TAB) && ui.get_active_tab() != m.tab as i32 {
        ui.set_active_tab(m.tab as i32);
    }
    if changed.has(PART_LOGS) {
        ui.set_log_text(m.logs.text.as_str().into());
        ui.set_log_container_name(m.logs.container.as_str().into());
        if ui.get_show_logs() != m.logs.open {
            ui.set_show_logs(m.logs.open);
        }
    }
    if changed.has(PART_AI) {
        ui.set_ai_is_working(m.ai.working);
        ui.set_ai_response(m.ai.response.as_str().into());
    }
    if changed.has(rail::PART) {
        ui.render_rail(&m.rail);
        // The reason the Mind cannot answer is said only where there is a log it would have
        // answered about, as this app always said it.
        if !crate::rail::has_logs(m) {
            ui.set_agent_unavailable(SharedString::new());
        }
    }
}

fn listing(ui: &ContainerManagerApp, m: &Model) {
    let containers: Vec<ContainerData> = m
        .containers
        .iter()
        .map(|c| ContainerData {
            id: c.id.as_str().into(),
            name: c.name.as_str().into(),
            image: c.image.as_str().into(),
            status: c.state.as_str().into(),
            status_text: c.status_text.as_str().into(),
            ports: c.ports.as_str().into(),
            created: c.created.as_str().into(),
            is_selected: false,
        })
        .collect();
    ui.set_containers(ModelRc::new(VecModel::from(containers)));
    ui.set_running_count(m.running() as i32);
    ui.set_stopped_count(m.stopped() as i32);
    ui.set_total_count(m.total() as i32);
    let images: Vec<ImageData> = m
        .images
        .iter()
        .map(|i| ImageData {
            title: plain_image_name(&i.repo_tag).into(),
            id: i.id.as_str().into(),
            repo_tag: i.repo_tag.as_str().into(),
            size_text: i.size_text.as_str().into(),
            created: i.created.as_str().into(),
        })
        .collect();
    ui.set_images(ModelRc::new(VecModel::from(images)));
    let volumes: Vec<VolumeData> = m
        .volumes
        .iter()
        .map(|v| VolumeData {
            name: v.name.as_str().into(),
            driver: v.driver.as_str().into(),
            mount_point: v.mount_point.as_str().into(),
            size_text: SharedString::new(),
        })
        .collect();
    ui.set_volumes(ModelRc::new(VecModel::from(volumes)));
    ui.set_runtime_name(m.runtime.as_str().into());
}
