use super::render::put_text;
use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let rows = agent_rows(endpoints, active_endpoint_id, config, false);
    for (index, row) in rows.into_iter().take(area.height as usize).enumerate() {
        let rect = Rect::new(area.x, area.y + index as u16, area.width, 1);
        if row.agent.focused {
            buffer.set_style(rect, Style::default().bg(config.palette.active_row_bg));
        }
        let initial = row.machine_label.chars().next().unwrap_or('?');
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{initial}{}",
                status_icon(row.agent.status, config.status_indicators)
            ),
            Style::default()
                .fg(if row.stale {
                    config.palette.overlay0
                } else {
                    status_color(row.agent.status, &config.palette)
                })
                .add_modifier(if row.stale {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                }),
        );
        hits.endpoint_agents
            .push((rect, row.endpoint_id, row.agent.pane_id));
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !super::agent_sidebar::render_agent_panel_header(
        buffer,
        area,
        agent_view_label,
        config,
        hits,
    ) {
        return;
    }
    let rows = agent_rows(
        endpoints,
        active_endpoint_id,
        config,
        super::agent_sidebar::agent_groups_enabled(config, agent_view_label),
    );
    let gaps = agent_gaps(&rows, config);
    super::agent_sidebar::render_agent_list(
        buffer,
        area,
        &rows,
        &gaps,
        agent_view_label.map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.agent.height(),
        |buffer, rect, row, hits| {
            super::agent_sidebar::render_agent_row(buffer, rect, &row.agent, config);
            if row.stale {
                buffer.set_style(
                    rect,
                    Style::default()
                        .fg(config.palette.overlay0)
                        .add_modifier(Modifier::DIM),
                );
            }
            let child = row.agent.child_rect(rect);
            if !child.is_empty() {
                hits.endpoint_agents.push((
                    child,
                    row.endpoint_id.clone(),
                    row.agent.pane_id.clone(),
                ));
            }
        },
    );
}

impl ClientShellState {
    pub(super) fn reveal_endpoint_agent(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
        body_height: u16,
    ) {
        if body_height == 0 {
            return;
        }
        let agent_view_label = self
            .snapshot
            .as_deref()
            .and_then(|snapshot| snapshot.agent_view_label.as_deref());
        let rows = agent_rows(
            &self.endpoints,
            &self.active_endpoint_id,
            &self.config,
            super::agent_sidebar::agent_groups_enabled(&self.config, agent_view_label),
        );
        let Some(target) = rows
            .iter()
            .position(|row| &row.endpoint_id == endpoint_id && row.agent.pane_id == pane_id)
        else {
            return;
        };
        let heights = rows
            .iter()
            .map(|row| row.agent.height().min(u16::MAX as usize) as u16)
            .collect::<Vec<_>>();
        let gaps = agent_gaps(&rows, &self.config);
        self.agent_scroll = super::scroll::list_scroll_start_to_reveal(
            &heights,
            &gaps,
            body_height,
            self.agent_scroll,
            target,
        );
    }
}

struct EndpointAgentRow {
    endpoint_id: ClientEndpointId,
    machine_label: String,
    stale: bool,
    agent: super::agent_sidebar::AgentRow,
}

pub(super) fn online_agent_targets(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    collapsed: bool,
) -> Vec<super::aggregate_navigation::AggregateAgentTarget> {
    let label = endpoints
        .iter()
        .find(|endpoint| &endpoint.endpoint_id == active_endpoint_id)
        .and_then(|endpoint| endpoint.snapshot.as_deref())
        .and_then(|snapshot| snapshot.agent_view_label.as_deref());
    agent_rows(
        endpoints,
        active_endpoint_id,
        config,
        !collapsed && super::agent_sidebar::agent_groups_enabled(config, label),
    )
    .into_iter()
    .filter(|row| !row.stale)
    .map(|row| super::aggregate_navigation::AggregateAgentTarget {
        endpoint_id: row.endpoint_id,
        pane_id: row.agent.pane_id,
    })
    .collect()
}

fn agent_gaps(rows: &[EndpointAgentRow], config: &ClientShellConfig) -> Vec<u16> {
    super::agent_sidebar::agent_row_gaps(rows, config.agents.row_gap, |row| &row.agent)
}

fn agent_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    grouped: bool,
) -> Vec<EndpointAgentRow> {
    let mut rendered_rows = endpoints
        .iter()
        .filter_map(|endpoint| {
            endpoint.snapshot.as_deref().map(|snapshot| {
                snapshot
                    .agents
                    .iter()
                    .filter_map(|agent| {
                        super::agent_sidebar::agent_row(
                            snapshot,
                            &agent.pane_id,
                            config,
                            Some(&endpoint.label),
                        )
                    })
                    .map(|agent| ((endpoint.endpoint_id.clone(), agent.pane_id.clone()), agent))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect::<HashMap<_, _>>();

    let mut rows = super::aggregate_navigation::aggregate_agent_rows(
        endpoints,
        active_endpoint_id,
        config.agent_panel_sort,
    )
    .into_iter()
    .filter_map(|row| {
        let key = (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone());
        let mut agent = rendered_rows.remove(&key)?;
        agent.focused &= row.endpoint.endpoint_id == active_endpoint_id;
        Some(EndpointAgentRow {
            endpoint_id: row.endpoint.endpoint_id.clone(),
            machine_label: row.endpoint.label.to_owned(),
            stale: row.endpoint.stale(),
            agent,
        })
    })
    .collect::<Vec<_>>();
    if grouped {
        rows.sort_by_cached_key(|row| {
            let endpoint = endpoints
                .iter()
                .position(|endpoint| endpoint.endpoint_id == row.endpoint_id);
            let workspace = endpoint
                .and_then(|index| endpoints[index].snapshot.as_deref())
                .and_then(|snapshot| {
                    snapshot
                        .workspaces
                        .iter()
                        .position(|workspace| workspace.workspace_id == row.agent.workspace_id)
                });
            (endpoint, workspace)
        });
        let boundaries = super::agent_sidebar::group_boundaries(&rows, |a, b| {
            a.endpoint_id == b.endpoint_id && a.agent.workspace_id == b.agent.workspace_id
        });
        for (row, (first, last)) in rows.iter_mut().zip(boundaries) {
            row.agent.group(first, last, Some(&row.machine_label));
        }
    }
    rows
}
