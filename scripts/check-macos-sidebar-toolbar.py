#!/usr/bin/env python3
"""Capture one owned native window and check conversation actions against its divider.

Run after an explicitly authorized SVC preview launch. This does not launch,
activate, resize, or terminate applications. Screenshots still require review.
"""
import argparse
import json
from pathlib import Path
import subprocess


def desktop(*arguments):
    result = subprocess.run(['agent-desktop', *map(str, arguments)], capture_output=True,
                            text=True, check=True, timeout=30)
    value = json.loads(result.stdout)
    if not value.get('ok'):
        raise RuntimeError(value.get('error'))
    return value


def nodes(tree):
    yield tree
    for child in tree.get('children', []):
        yield from nodes(child)


def check(tree, placement, top_offset=150, surface='sidebar-titlebar'):
    all_nodes = list(nodes(tree))
    # NSToolbar AX wrappers repeat their child's label; count actual controls.
    controls = [node for node in all_nodes if not any(
        child.get('name') == node.get('name') for child in node.get('children', []))]
    # SwiftUI can expose this Menu by its observed SF Symbol AXIdentifier
    # after the sidebar is restored; this is an identity alias, not geometry.
    filter_ids = {'line.3.horizontal.decrease', 'line.3.horizontal.decrease.circle.fill'}
    found = {'显示': [node for node in controls if node.get('name') == '显示' or
                    (node.get('role') == 'combobox' and node.get('native_id', {}).get('value') in filter_ids)],
             '新建会话': [node for node in controls if node.get('name') == '新建会话']}
    fields = [node for node in controls if node.get('role') == 'textfield' and node.get('name') == '搜索会话']
    search_buttons = [node for node in controls if node.get('role') == 'button' and node.get('name') == '搜索会话']
    toolbars = [node for node in all_nodes if node.get('role') == 'toolbar']
    toolbar_node_ids = {id(node) for toolbar in toolbars for node in nodes(toolbar)}
    toolbar_rects = [node['bounds'] for node in toolbars if node.get('bounds')]
    dividers = [node['bounds'] for node in all_nodes if node.get('role') == 'splitter'
                and 'offscreen' not in node.get('states', []) and node.get('bounds')]
    hidden = surface == 'hidden' or placement == 'absent'
    result = {'surface': surface, 'placement': placement, 'actions': found, 'searchFields': fields,
              'searchButtons': search_buttons, 'dividers': dividers, 'toolbarBounds': toolbar_rects,
              'failures': [], 'requiresDividerVisualReview': True}
    failures = result['failures']
    for name, matches in found.items():
        expected = 0 if hidden else 1
        if len(matches) != expected:
            failures.append(f'{name}: expected {expected}, got {len(matches)}')
    expected_fields = 0 if hidden or surface == 'shared-sidebar' else 1
    if len(fields) != expected_fields:
        failures.append(f'Search fields: expected {expected_fields}, got {len(fields)}')
    if hidden and search_buttons:
        failures.append('Hidden sidebar must not retain a list search button')
    if surface == 'shared-sidebar' and len(search_buttons) != 1:
        failures.append(f'Shared header search buttons: expected 1, got {len(search_buttons)}')

    def within_toolbar(node):
        rect = node.get('bounds')
        return rect and id(node) in toolbar_node_ids and any(
            rect['y'] >= bar['y'] - 1 and rect['y'] + rect['height'] <= bar['y'] + bar['height'] + 1
            for bar in toolbar_rects)

    if not hidden and surface in ('sidebar-titlebar', 'shared-sidebar'):
        if not toolbar_rects:
            failures.append('No native window toolbar bounds')
        for name, matches in found.items():
            for node in matches:
                expected_toolbar = surface == 'sidebar-titlebar' or name == '新建会话'
                if bool(within_toolbar(node)) != expected_toolbar:
                    failures.append(f'{name}: expected native toolbar={expected_toolbar}, got bounds={node.get("bounds")}')
        if placement not in ('left', 'right'):
            failures.append('Sidebar surface requires an explicit left/right divider')
    if not hidden and placement in ('left', 'right'):
        if not dividers:
            failures.append('No visible sidebar/content divider')
        else:
            divider = min(d['x'] for d in dividers) if placement == 'left' else max(d['x'] for d in dividers)
            for name, matches in list(found.items()) + [('搜索会话', fields + search_buttons)]:
                for node in matches:
                    rect = node.get('bounds')
                    inside = rect and (rect['x'] + rect['width'] <= divider + 1 if placement == 'left' else rect['x'] >= divider)
                    if not inside:
                        failures.append(f'{name}: outside {placement} sidebar: {rect}, divider={divider}')
    if surface == 'sidebar-titlebar' and not hidden:
        toolbar_bottom = max((bar['y'] + bar['height'] for bar in toolbar_rects), default=0)
        for field in fields:
            rect = field.get('bounds')
            if not rect or rect['y'] < toolbar_bottom - 1 or id(field) in toolbar_node_ids:
                failures.append(f'Search must be in sidebar content below titlebar: {rect}')
            for matches in found.values():
                for action in matches:
                    bounds = action.get('bounds')
                    if rect and bounds and rect['y'] < bounds['y'] + bounds['height'] - 1:
                        failures.append('Search field must not share the titlebar action row')
    if surface == 'shared-sidebar' and not hidden:
        toolbar_bottom = max((bar['y'] + bar['height'] for bar in toolbar_rects), default=0)
        header_controls = found['显示'] + search_buttons
        for node in header_controls:
            rect = node.get('bounds')
            if not rect or rect['y'] < toolbar_bottom - 1 or id(node) in toolbar_node_ids:
                failures.append(f'Shared search/filter must remain in the content ToolHeader: {rect}')
        if len(header_controls) == 2:
            first, second = [node.get('bounds') for node in header_controls]
            if first and second and (first['y'] >= second['y'] + second['height'] or second['y'] >= first['y'] + first['height']):
                failures.append('Shared search/filter must share the ToolHeader row')
    if not hidden and surface == 'panel':
        for matches in found.values():
            for node in matches:
                if id(node) in toolbar_node_ids:
                    failures.append('Detached/panel actions must remain in their own content header')
    top = tree.get('bounds', {}).get('y')
    if top is not None and not hidden:
        for node in [node for matches in found.values() for node in matches] + fields + search_buttons:
            rect = node.get('bounds')
            if not rect or rect['y'] + rect['height'] > top + top_offset:
                failures.append(f'{node.get("name")}: outside top surface area: {rect}')
    result['toolbarNodes'] = [{key: node[key] for key in ('role', 'name', 'native_id', 'bounds') if key in node}
                              for node in controls if id(node) in toolbar_node_ids]
    result['toolbarSeparators'] = [node for toolbar in toolbars for node in nodes(toolbar)
                                   if node.get('role') in ('separator', 'splitter')]
    result['toolbarIdentifierAvailability'] = 'AX native_id only; underlying NSToolbarItem identifiers unavailable'
    result['passed'] = not failures
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', required=True, help='Exact development bundle identifier')
    parser.add_argument('--pid', required=True, type=int, help='PID from current SVC receipt')
    parser.add_argument('--window-id', required=True)
    parser.add_argument('--placement', choices=['left', 'right', 'absent', 'any'], default='left')
    parser.add_argument('--surface', choices=['sidebar-titlebar', 'shared-sidebar', 'panel', 'hidden'], default='sidebar-titlebar', help='Expected historical list surface; titlebar actions and content search stay separate')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--top-offset', type=float, default=150, help='Maximum action bottom relative to window top')
    args = parser.parse_args()
    if not args.app.startswith('local.velune.development.'):
        parser.error('Only a development application may be checked')
    windows = desktop('list-windows', '--app', args.app)['data']
    selected = [window for window in windows if window['id'] == args.window_id]
    if len(selected) != 1 or selected[0]['pid'] != args.pid:
        parser.error('Window is not owned by the specified current PID')
    args.output.mkdir(parents=True, exist_ok=True)
    scope = ['--app', args.app, '--window-id', args.window_id]
    readiness = ['--wait-for-gone', 'button:新建会话'] if args.surface == 'hidden' or args.placement == 'absent' else ['--wait-for', 'button:新建会话']
    snapshot = desktop('snapshot', *scope, '--include-bounds', '--max-depth', '24', '--timeout-ms', '15000', *readiness)
    (args.output / 'ax.json').write_text(json.dumps(snapshot, ensure_ascii=False, indent=2) + '\n')
    if not snapshot['data'].get('complete'):
        raise RuntimeError('Incomplete accessibility tree cannot establish action counts')
    screenshot = desktop('screenshot', *scope, args.output / 'window.png')
    result = check(snapshot['data']['tree'], args.placement, args.top_offset, args.surface)
    result.update(window=selected[0], snapshotWindowBounds=snapshot['data']['tree'].get('bounds'),
                  screenshot=screenshot['data'], requiresVisualReview=True)
    (args.output / 'result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False))
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
