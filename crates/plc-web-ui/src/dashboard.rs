//! Embedded HTML dashboard for PLC monitoring.
//!
//! Provides a single-page dashboard that connects via WebSocket
//! to display real-time PLC state.

use axum::response::Html;

/// Dashboard HTML content.
///
/// This is an embedded dashboard that uses htmx-style updates via WebSocket.
/// No build step required - pure HTML/CSS/JS.
const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Virtual PLC Dashboard</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            color: #eee;
            min-height: 100vh;
        }
        .header {
            background: #16213e;
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 2px solid #0f3460;
        }
        .header h1 { font-size: 1.5rem; font-weight: 600; }
        .status-badge {
            padding: 0.25rem 0.75rem;
            border-radius: 9999px;
            font-size: 0.875rem;
            font-weight: 500;
        }
        .status-run { background: #10b981; color: #fff; }
        .status-init { background: #f59e0b; color: #000; }
        .status-fault { background: #ef4444; color: #fff; }
        .status-stop { background: #6b7280; color: #fff; }
        .status-boot { background: #8b5cf6; color: #fff; }
        .status-pre_op { background: #f59e0b; color: #000; }
        .status-safe_stop { background: #fb923c; color: #000; }
        .status-disconnected, .status-unknown { background: #374151; color: #9ca3af; }

        .container { padding: 1.5rem 2rem; }

        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1.5rem;
        }

        .card {
            background: #16213e;
            border-radius: 0.5rem;
            border: 1px solid #0f3460;
            overflow: hidden;
        }
        .card-header {
            background: #0f3460;
            padding: 0.75rem 1rem;
            font-weight: 600;
            font-size: 0.875rem;
            text-transform: uppercase;
            letter-spacing: 0.05em;
        }
        .card-body { padding: 1rem; }

        .metric-grid {
            display: grid;
            grid-template-columns: repeat(2, 1fr);
            gap: 1rem;
        }
        .metric {
            text-align: center;
            padding: 0.5rem;
        }
        .metric-value {
            font-size: 1.5rem;
            font-weight: 700;
            color: #60a5fa;
        }
        .metric-label {
            font-size: 0.75rem;
            color: #9ca3af;
            margin-top: 0.25rem;
        }

        .io-grid {
            display: grid;
            grid-template-columns: repeat(8, 1fr);
            gap: 0.5rem;
        }
        .io-bit {
            width: 100%;
            aspect-ratio: 1;
            border-radius: 0.25rem;
            display: flex;
            align-items: center;
            justify-content: center;
            font-size: 0.625rem;
            font-weight: 600;
        }
        .io-bit.on { background: #10b981; color: #fff; }
        .io-bit.off { background: #374151; color: #6b7280; }

        .analog-bar {
            height: 1.5rem;
            background: #374151;
            border-radius: 0.25rem;
            overflow: hidden;
            margin-bottom: 0.5rem;
            position: relative;
        }
        .analog-bar-fill {
            height: 100%;
            background: linear-gradient(90deg, #3b82f6, #60a5fa);
            transition: width 0.2s ease;
        }
        .analog-bar-label {
            position: absolute;
            right: 0.5rem;
            top: 50%;
            transform: translateY(-50%);
            font-size: 0.75rem;
            font-weight: 500;
        }

        .fault-list {
            max-height: 200px;
            overflow-y: auto;
        }
        .fault-item {
            padding: 0.5rem;
            border-bottom: 1px solid #0f3460;
            font-size: 0.875rem;
        }
        .fault-item:last-child { border-bottom: none; }
        .fault-time { color: #9ca3af; font-size: 0.75rem; }
        .fault-reason { color: #f87171; }

        .connection-status {
            display: flex;
            align-items: center;
            gap: 0.5rem;
            font-size: 0.875rem;
        }
        .connection-dot {
            width: 0.5rem;
            height: 0.5rem;
            border-radius: 50%;
        }
        .connection-dot.connected { background: #10b981; }
        .connection-dot.disconnected { background: #ef4444; animation: pulse 2s infinite; }
        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.5; }
        }
        .no-faults { color: #6b7280; }
    </style>
</head>
<body>
    <div class="header">
        <h1>Virtual PLC Dashboard</h1>
        <div style="display: flex; align-items: center; gap: 1rem;">
            <div class="connection-status">
                <div class="connection-dot disconnected" id="conn-dot"></div>
                <span id="conn-text">Connecting...</span>
            </div>
            <span id="runtime-state" class="status-badge status-disconnected">--</span>
        </div>
    </div>

    <div class="container">
        <div class="grid">
            <!-- Cycle Metrics -->
            <div class="card">
                <div class="card-header">Cycle Metrics</div>
                <div class="card-body">
                    <div class="metric-grid">
                        <div class="metric">
                            <div class="metric-value" id="m-cycles">0</div>
                            <div class="metric-label">Total Cycles</div>
                        </div>
                        <div class="metric">
                            <div class="metric-value" id="m-avg">0</div>
                            <div class="metric-label">Avg Cycle (us)</div>
                        </div>
                        <div class="metric">
                            <div class="metric-value" id="m-max">0</div>
                            <div class="metric-label">Max Cycle (us)</div>
                        </div>
                        <div class="metric">
                            <div class="metric-value" id="m-jitter">0</div>
                            <div class="metric-label">Jitter (us)</div>
                        </div>
                        <div class="metric">
                            <div class="metric-value" id="m-target">0</div>
                            <div class="metric-label">Target (us)</div>
                        </div>
                        <div class="metric">
                            <div class="metric-value" id="m-overruns" style="color: #f87171;">0</div>
                            <div class="metric-label">Overruns</div>
                        </div>
                    </div>
                </div>
            </div>

            <!-- Digital Inputs -->
            <div class="card">
                <div class="card-header">Digital Inputs</div>
                <div class="card-body">
                    <div class="io-grid" id="di-grid"></div>
                </div>
            </div>

            <!-- Digital Outputs -->
            <div class="card">
                <div class="card-header">Digital Outputs</div>
                <div class="card-body">
                    <div class="io-grid" id="do-grid"></div>
                </div>
            </div>

            <!-- Analog I/O -->
            <div class="card">
                <div class="card-header">Analog Inputs</div>
                <div class="card-body" id="ai-container"></div>
            </div>

            <div class="card">
                <div class="card-header">Analog Outputs</div>
                <div class="card-body" id="ao-container"></div>
            </div>

            <!-- Faults -->
            <div class="card">
                <div class="card-header">Recent Faults</div>
                <div class="card-body">
                    <div class="fault-list" id="fault-list">
                        <div class="fault-item no-faults">No faults recorded</div>
                    </div>
                </div>
            </div>
        </div>
    </div>

    <script>
        // Create an element with given tag, className, id, and textContent
        function createElement(tag, opts) {
            const el = document.createElement(tag);
            if (opts.className) el.className = opts.className;
            if (opts.id) el.id = opts.id;
            if (opts.text) el.textContent = opts.text;
            if (opts.style) el.style.cssText = opts.style;
            return el;
        }

        // Initialize digital I/O grids using safe DOM methods
        function initIoGrids() {
            const diGrid = document.getElementById('di-grid');
            const doGrid = document.getElementById('do-grid');
            for (let i = 0; i < 32; i++) {
                diGrid.appendChild(createElement('div', {
                    className: 'io-bit off',
                    id: 'di-' + i,
                    text: String(i)
                }));
                doGrid.appendChild(createElement('div', {
                    className: 'io-bit off',
                    id: 'do-' + i,
                    text: String(i)
                }));
            }
        }

        // Initialize analog bars using safe DOM methods
        function initAnalogBars() {
            const aiContainer = document.getElementById('ai-container');
            const aoContainer = document.getElementById('ao-container');

            for (let i = 0; i < 4; i++) {
                // Create AI bar
                const aiBar = createElement('div', { className: 'analog-bar' });
                aiBar.appendChild(createElement('div', {
                    className: 'analog-bar-fill',
                    id: 'ai-' + i,
                    style: 'width: 50%'
                }));
                aiBar.appendChild(createElement('span', {
                    className: 'analog-bar-label',
                    id: 'ai-' + i + '-val',
                    text: 'AI' + i + ': 0'
                }));
                aiContainer.appendChild(aiBar);

                // Create AO bar
                const aoBar = createElement('div', { className: 'analog-bar' });
                aoBar.appendChild(createElement('div', {
                    className: 'analog-bar-fill',
                    id: 'ao-' + i,
                    style: 'width: 50%'
                }));
                aoBar.appendChild(createElement('span', {
                    className: 'analog-bar-label',
                    id: 'ao-' + i + '-val',
                    text: 'AO' + i + ': 0'
                }));
                aoContainer.appendChild(aoBar);
            }
        }

        // Update digital I/O display
        function updateDigitalIo(di, doVal) {
            for (let i = 0; i < 32; i++) {
                const diBit = document.getElementById('di-' + i);
                const doBit = document.getElementById('do-' + i);
                if (diBit) {
                    diBit.className = (di & (1 << i)) ? 'io-bit on' : 'io-bit off';
                }
                if (doBit) {
                    doBit.className = (doVal & (1 << i)) ? 'io-bit on' : 'io-bit off';
                }
            }
        }

        // Update analog display
        function updateAnalog(ai, ao) {
            for (let i = 0; i < 4; i++) {
                const aiVal = ai[i] || 0;
                const aoVal = ao[i] || 0;
                const aiPct = Math.abs(aiVal) / 32768 * 100;
                const aoPct = Math.abs(aoVal) / 32768 * 100;

                const aiBar = document.getElementById('ai-' + i);
                const aoBar = document.getElementById('ao-' + i);
                if (aiBar) aiBar.style.width = aiPct + '%';
                if (aoBar) aoBar.style.width = aoPct + '%';

                const aiLabel = document.getElementById('ai-' + i + '-val');
                const aoLabel = document.getElementById('ao-' + i + '-val');
                if (aiLabel) aiLabel.textContent = 'AI' + i + ': ' + aiVal;
                if (aoLabel) aoLabel.textContent = 'AO' + i + ': ' + aoVal;
            }
        }

        // Update metrics display
        function updateMetrics(m) {
            const cycles = m.total_cycles ? m.total_cycles.toLocaleString() : '0';
            document.getElementById('m-cycles').textContent = cycles;
            document.getElementById('m-avg').textContent = m.avg_us || '0';
            document.getElementById('m-max').textContent = m.max_us || '0';
            document.getElementById('m-jitter').textContent = m.jitter_us || '0';
            document.getElementById('m-target').textContent = m.target_us || '0';
            document.getElementById('m-overruns').textContent = m.overrun_count || '0';
        }

        // Update state badge
        function updateState(state) {
            const badge = document.getElementById('runtime-state');
            badge.textContent = state;
            // Handle disconnect state ('--') and normalize to CSS class
            const stateClass = state === '--' ? 'unknown' : state.toLowerCase();
            badge.className = 'status-badge status-' + stateClass;
        }

        // Add fault to list using safe DOM methods
        function addFault(fault) {
            const list = document.getElementById('fault-list');
            // Clear "no faults" message
            const noFaults = list.querySelector('.no-faults');
            if (noFaults) {
                list.removeChild(noFaults);
            }

            const item = createElement('div', { className: 'fault-item' });

            const timeSpan = createElement('span', {
                className: 'fault-time',
                text: 'Cycle ' + fault.cycle + ' @ ' + fault.timestamp_ms + 'ms'
            });
            item.appendChild(timeSpan);
            item.appendChild(document.createElement('br'));

            const reasonSpan = createElement('span', {
                className: 'fault-reason',
                text: fault.reason
            });
            item.appendChild(reasonSpan);

            list.insertBefore(item, list.firstChild);

            // Keep only last 10 faults
            while (list.children.length > 10) {
                list.removeChild(list.lastChild);
            }
        }

        // WebSocket connection
        let ws = null;
        let reconnectTimer = null;

        function connect() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = protocol + '//' + window.location.host + '/ws';

            ws = new WebSocket(wsUrl);

            ws.onopen = function() {
                document.getElementById('conn-dot').className = 'connection-dot connected';
                document.getElementById('conn-text').textContent = 'Connected';
                if (reconnectTimer) {
                    clearInterval(reconnectTimer);
                    reconnectTimer = null;
                }
            };

            ws.onclose = function() {
                document.getElementById('conn-dot').className = 'connection-dot disconnected';
                document.getElementById('conn-text').textContent = 'Disconnected';
                updateState('--');
                if (!reconnectTimer) {
                    reconnectTimer = setInterval(connect, 3000);
                }
            };

            ws.onerror = function() {
                ws.close();
            };

            ws.onmessage = function(event) {
                try {
                    const msg = JSON.parse(event.data);

                    if (msg.type === 'full') {
                        updateState(msg.runtime_state || 'Unknown');
                        if (msg.io) {
                            updateDigitalIo(msg.io.digital_inputs, msg.io.digital_outputs);
                            updateAnalog(msg.io.analog_inputs || [], msg.io.analog_outputs || []);
                        }
                        if (msg.metrics) updateMetrics(msg.metrics);
                        if (msg.faults) {
                            msg.faults.slice(-5).forEach(addFault);
                        }
                    } else if (msg.type === 'io') {
                        updateDigitalIo(msg.digital_inputs, msg.digital_outputs);
                        updateAnalog(msg.analog_inputs || [], msg.analog_outputs || []);
                    } else if (msg.type === 'metrics') {
                        updateMetrics(msg);
                    } else if (msg.type === 'state') {
                        updateState(msg.state);
                    } else if (msg.type === 'fault') {
                        addFault(msg);
                    }
                } catch (e) {
                    console.error('Failed to parse WebSocket message:', e);
                }
            };
        }

        // Initialize on load
        initIoGrids();
        initAnalogBars();
        connect();
    </script>
</body>
</html>
"#;

/// Serve the embedded dashboard HTML.
///
/// GET /
/// GET /dashboard
pub async fn dashboard_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}
