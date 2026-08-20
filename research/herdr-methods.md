# Herdr socket API — full method catalog (protocol 20, herdr 0.8.2)

- `ping` {}
- `server.stop` {}
- `server.live_handoff` {?expected_protocol:integer|null, ?expected_version:string|null, ?import_exe:string|null}
- `server.reload_config` {}
- `server.agent_manifests` {}
- `server.reload_agent_manifests` {}
- `notification.show` {?body:string|null, ?position:ToastHerdrPosition|null, ?sound:NotificationShowSound, title:string}
- `client.window_title.set` {title:string}
- `client.window_title.clear` {}
- `session.snapshot` {}
- `workspace.create` {?cwd:string|null, ?env:object, ?focus:boolean, ?label:string|null}
- `workspace.list` {}
- `workspace.get` {workspace_id:string}
- `workspace.focus` {workspace_id:string}
- `workspace.rename` {label:string, workspace_id:string}
- `workspace.move` {insert_index:integer, workspace_id:string}
- `workspace.move_block` {?before_workspace_id:string|null, workspace_ids:array}
- `workspace.report_metadata` {?seq:integer|null, source:string, tokens:object, ?ttl_ms:integer|null, workspace_id:string}
- `workspace.close` {workspace_id:string}
- `worktree.list` {?cwd:string|null, ?workspace_id:string|null}
- `worktree.create` {?base:string|null, ?branch:string|null, ?cwd:string|null, ?focus:boolean, ?label:string|null, ?path:string|null, ?workspace_id:string|null}
- `worktree.open` {?branch:string|null, ?cwd:string|null, ?focus:boolean, ?label:string|null, ?path:string|null, ?workspace_id:string|null}
- `worktree.remove` {?force:boolean, workspace_id:string}
- `tab.create` {?cwd:string|null, ?env:object, ?focus:boolean, ?label:string|null, ?workspace_id:string|null}
- `tab.list` {?workspace_id:string|null}
- `tab.get` {tab_id:string}
- `tab.focus` {tab_id:string}
- `tab.rename` {label:string, tab_id:string}
- `tab.move` {insert_index:integer, tab_id:string}
- `tab.close` {tab_id:string}
- `agent.list` {}
- `agent.get` {target:string}
- `agent.read` {?format:ReadFormat, ?lines:integer|null, source:ReadSource, ?strip_ansi:boolean, target:string}
- `agent.explain` {target:string}
- `agent.send_keys` {keys:array, target:string}
- `agent.rename` {?name:string|null, target:string}
- `agent.view.set` {?filter:AgentViewFilter|null, ?label:string|null, ?sort:array, source:string}
- `agent.view.clear` {?source:string|null}
- `agent.focus` {target:string}
- `agent.start` {?args:array, kind:string, name:string, pane_id:string, ?timeout_ms:integer|null}
- `agent.prompt` {target:string, text:string, ?wait:AgentPromptWaitOptions|null}
- `agent.wait` {target:string, ?timeout_ms:integer|null, ?until:array}
- `pane.split` {?cwd:string|null, direction:SplitDirection, ?env:object, ?focus:boolean, ?ratio:number|null, ?right_click:PaneRightClickTarget, ?target_pane_id:string|null, ?workspace_id:string|null}
- `pane.swap` {?direction:PaneDirection|null, ?pane_id:string|null, ?source_pane_id:string|null, ?target_pane_id:string|null}
- `pane.move` {destination:PaneMoveDestination, ?focus:boolean, pane_id:string}
- `pane.zoom` {?mode:PaneZoomMode, ?pane_id:string|null}
- `pane.layout` {?pane_id:string|null}
- `pane.process_info` {?pane_id:string|null}
- `layout.export` {?pane_id:string|null, ?tab_id:string|null}
- `layout.apply` {?focus:boolean, root:LayoutNode, ?tab_id:string|null, ?tab_label:string|null, ?workspace_id:string|null}
- `layout.set_split_ratio` {?pane_id:string|null, path:array, ratio:number, ?tab_id:string|null}
- `pane.neighbor` {direction:PaneDirection, ?pane_id:string|null}
- `pane.edges` {?pane_id:string|null}
- `pane.focus_direction` {direction:PaneDirection, ?pane_id:string|null}
- `pane.resize` {?amount:number|null, direction:PaneDirection, ?pane_id:string|null}
- `pane.list` {?workspace_id:string|null}
- `pane.current` {?caller_pane_id:string|null}
- `pane.get` {pane_id:string}
- `pane.focus` {pane_id:string}
- `pane.input.set` {pane_id:string, right_click:PaneRightClickTarget}
- `pane.rename` {?label:string|null, pane_id:string}
- `pane.send_text` {pane_id:string, text:string}
- `pane.send_keys` {keys:array, pane_id:string}
- `pane.send_input` {?keys:array, pane_id:string, ?text:string}
- `pane.read` {?format:ReadFormat, ?lines:integer|null, pane_id:string, source:ReadSource, ?strip_ansi:boolean}
- `pane.graphics.set` {?data_base64:string, format:PaneGraphicsFormat, image_height:integer, image_width:integer, ?layer_id:string|null, pane_id:string, ?placement:PaneGraphicsPlacementParams, ?z_index:integer}
- `pane.graphics.clear` {?layer_id:string|null, pane_id:string}
- `pane.graphics.info` {pane_id:string}
- `pane.report_agent` {agent:string, ?agent_session_id:string|null, ?agent_session_path:string|null, ?message:string|null, pane_id:string, ?seq:integer|null, source:string, state:PaneAgentState}
- `pane.report_agent_session` {agent:string, ?agent_session_id:string|null, ?agent_session_path:string|null, pane_id:string, ?seq:integer|null, ?session_start_source:string|null, source:string}
- `pane.report_metadata` {?agent:string|null, ?applies_to_source:string|null, ?clear_display_agent:boolean, ?clear_state_labels:boolean, ?clear_title:boolean, ?display_agent:string|null, pane_id:string, ?seq:integer|null, source:string, ?state_labels:object, ?title:string|null, ?tokens:object, ?ttl_ms:integer|null}
- `pane.clear_agent_authority` {pane_id:string, ?seq:integer|null, ?source:string|null}
- `pane.release_agent` {agent:string, pane_id:string, ?seq:integer|null, source:string}
- `pane.close` {pane_id:string}
- `popup.close` {}
- `events.subscribe` {subscriptions:array}
- `events.wait` {match_event:EventMatch, ?timeout_ms:integer|null}
- `pane.wait_for_output` {?lines:integer|null, match:OutputMatch, pane_id:string, source:ReadSource, ?strip_ansi:boolean, ?timeout_ms:integer|null}
- `integration.install` {target:IntegrationTarget}
- `integration.uninstall` {target:IntegrationTarget}
- `plugin.link` {?enabled:boolean, path:string, ?source:PluginSourceInfo|null}
- `plugin.list` {?plugin_id:string|null}
- `plugin.unlink` {plugin_id:string}
- `plugin.enable` {plugin_id:string}
- `plugin.disable` {plugin_id:string}
- `plugin.action.list` {?plugin_id:string|null}
- `plugin.action.invoke` {action_id:string, ?context:PluginInvocationContext|null, ?plugin_id:string|null}
- `plugin.log.list` {?limit:integer|null, ?plugin_id:string|null}
- `plugin.pane.open` {?cwd:string|null, ?direction:SplitDirection|null, entrypoint:string, ?env:object, ?focus:boolean, ?height:PopupSize|null, ?placement:PluginPanePlacement|null, plugin_id:string, ?target_pane_id:string|null, ?width:PopupSize|null, ?workspace_id:string|null}
- `plugin.pane.focus` {pane_id:string}
- `plugin.pane.close` {pane_id:string}


# Events


## event kinds

`workspace_created`, `workspace_updated`, `workspace_metadata_updated`, `workspace_closed`, `workspace_renamed`, `workspace_moved`, `workspace_reordered`, `workspace_focused`, `worktree_created`, `worktree_opened`, `worktree_removed`, `tab_created`, `tab_closed`, `tab_renamed`, `tab_moved`, `tab_focused`, `pane_created`, `pane_closed`, `pane_updated`, `pane_focused`, `pane_moved`, `pane_output_changed`, `pane_exited`, `pane_agent_detected`, `pane_agent_status_changed`, `layout_updated`

Payload variants: 26

## subscription_event kinds

`pane.output_matched`, `pane.agent_status_changed`, `pane.scroll_changed`

Payload variants: 3
