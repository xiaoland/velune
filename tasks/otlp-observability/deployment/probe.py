"""sfp7 后台的合成验收探针；不属于 Velune 产品 exporter。"""

import argparse
import json
import time
import uuid
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import Request, urlopen

import grpc
from opentelemetry.proto.common.v1.common_pb2 import AnyValue, KeyValue
from opentelemetry.proto.resource.v1.resource_pb2 import Resource
from opentelemetry.proto.trace.v1.trace_pb2 import ResourceSpans, ScopeSpans, Span, Status
from opentelemetry.proto.logs.v1.logs_pb2 import ResourceLogs, ScopeLogs, LogRecord
from opentelemetry.proto.metrics.v1.metrics_pb2 import ResourceMetrics, ScopeMetrics, Metric, Sum, NumberDataPoint, Histogram, HistogramDataPoint
from opentelemetry.proto.collector.trace.v1.trace_service_pb2 import ExportTraceServiceRequest
from opentelemetry.proto.collector.trace.v1.trace_service_pb2_grpc import TraceServiceStub
from opentelemetry.proto.collector.logs.v1.logs_service_pb2 import ExportLogsServiceRequest
from opentelemetry.proto.collector.logs.v1.logs_service_pb2_grpc import LogsServiceStub
from opentelemetry.proto.collector.metrics.v1.metrics_service_pb2 import ExportMetricsServiceRequest
from opentelemetry.proto.collector.metrics.v1.metrics_service_pb2_grpc import MetricsServiceStub

ROOT = Path(__file__).resolve().parents[3] / 'target' / 'otlp-backend-probe'


def attr(key, value):
    return KeyValue(key=key, value=AnyValue(string_value=value))


def resource(role):
    return Resource(attributes=[attr('service.name', 'velune-probe-' + role), attr('service.version', 'backend-verification-1')])


def query(datasource, path, params=None):
    token = json.loads((ROOT / 'query-token.json').read_text())['key']
    url = 'http://127.0.0.1:30300/api/datasources/proxy/uid/' + datasource + path
    if params:
        url += '?' + urlencode(params)
    request = Request(url, headers={'Authorization': 'Bearer ' + token, 'Accept': 'application/json'})
    with urlopen(request, timeout=10) as response:
        return json.load(response)


def send(state, sequence=1):
    now = time.time_ns()
    trace_id = bytes.fromhex(state['trace_id'])
    parent_id = bytes.fromhex('0000000000000001')
    child_id = bytes.fromhex('0000000000000002')
    labels = [attr('probe_run_id', state['run_id'])]
    traces = ExportTraceServiceRequest(resource_spans=[
        ResourceSpans(resource=resource('controller'), scope_spans=[ScopeSpans(spans=[Span(
            trace_id=trace_id, span_id=parent_id, name='probe.send', kind=3,
            start_time_unix_nano=now - 50_000_000, end_time_unix_nano=now,
            attributes=labels, status=Status(code=1))])]),
        ResourceSpans(resource=resource('executor'), scope_spans=[ScopeSpans(spans=[Span(
            trace_id=trace_id, span_id=child_id, parent_span_id=parent_id, name='probe.execute', kind=2,
            start_time_unix_nano=now - 40_000_000, end_time_unix_nano=now - 10_000_000,
            attributes=labels, status=Status(code=1))])]),
    ])
    logs = ExportLogsServiceRequest(resource_logs=[ResourceLogs(resource=resource('executor'), scope_logs=[ScopeLogs(log_records=[LogRecord(
        time_unix_nano=now, observed_time_unix_nano=now, severity_number=9, severity_text='INFO',
        body=AnyValue(string_value='velune-probe ' + state['run_id']), attributes=labels,
        trace_id=trace_id, span_id=child_id, flags=1)])])])
    metrics = ExportMetricsServiceRequest(resource_metrics=[ResourceMetrics(resource=resource('executor'), scope_metrics=[ScopeMetrics(metrics=[
        Metric(name='velune_probe_operations', sum=Sum(aggregation_temporality=2, is_monotonic=True, data_points=[NumberDataPoint(
            attributes=labels, start_time_unix_nano=state['start_ns'], time_unix_nano=now, as_int=7 * sequence)])),
        Metric(name='velune_probe_latency', unit='s', histogram=Histogram(aggregation_temporality=2, data_points=[HistogramDataPoint(
            attributes=labels, start_time_unix_nano=state['start_ns'], time_unix_nano=now,
            count=3 * sequence, sum=0.35 * sequence, explicit_bounds=[0.1, 0.2], bucket_counts=[2 * sequence, sequence, 0])])),
    ])])])
    with grpc.insecure_channel('127.0.0.1:34317') as channel:
        for stub, request, field in [(TraceServiceStub, traces, 'rejected_spans'), (LogsServiceStub, logs, 'rejected_log_records'), (MetricsServiceStub, metrics, 'rejected_data_points')]:
            response = stub(channel).Export(request, timeout=10)
            assert getattr(response.partial_success, field) == 0, response
    state['sent_ns'] = now
    state['sequence'] = sequence


def verify(state):
    trace = query('tempo', '/api/traces/' + state['trace_id'])
    batches = trace.get('batches', trace.get('resourceSpans', []))
    services = {a['value']['stringValue'] for r in batches for a in r['resource']['attributes'] if a['key'] == 'service.name'}
    assert services == {'velune-probe-controller', 'velune-probe-executor'}, services
    spans = [s for r in batches for scope in r.get('scopeSpans', r.get('instrumentationLibrarySpans', [])) for s in scope.get('spans', [])]
    by_name = {s['name']: s for s in spans}
    assert set(by_name) == {'probe.send', 'probe.execute'}, trace
    assert by_name['probe.execute']['parentSpanId'] == by_name['probe.send']['spanId'], spans
    logs = query('loki', '/loki/api/v1/query_range', {
        'query': '{service_name="velune-probe-executor"} |= "' + state['run_id'] + '"',
        'start': state['start_ns'], 'end': time.time_ns(), 'limit': 10,
    })
    assert logs['data']['result'], logs
    assert state['trace_id'] in json.dumps(logs), logs
    selector = '{probe_run_id="' + state['run_id'] + '"}'
    values = {}
    for name, expected in [('velune_probe_operations_total', 7), ('velune_probe_latency_seconds_count', 3), ('velune_probe_latency_seconds_sum', 0.35)]:
        result = query('prometheus', '/api/v1/query', {'query': name + selector, 'time': state['sent_ns'] / 1e9 + 1})
        data = result['data']['result']
        assert len(data) == 1, result
        values[name] = float(data[0]['value'][1])
        assert abs(values[name] - expected * state['sequence']) < 0.00001, values
    return {'run_id': state['run_id'], 'trace_id': state['trace_id'], 'span_count': len(spans), 'parent_child': True, 'log_trace_correlation': True, 'metrics': values}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['send', 'verify'])
    parser.add_argument('state', type=Path)
    args = parser.parse_args()
    if args.mode == 'send':
        state = {'run_id': 'probe-' + uuid.uuid4().hex, 'trace_id': uuid.uuid4().hex, 'start_ns': time.time_ns() - 1_000_000_000}
        send(state)
        args.state.write_text(json.dumps(state))
        print(json.dumps({'accepted': True, **state}))
    else:
        print(json.dumps(verify(json.loads(args.state.read_text()))))
