import React, { useState, useEffect, useCallback } from 'react';
import {
  StyleSheet,
  Text,
  View,
  TextInput,
  TouchableOpacity,
  FlatList,
  ScrollView,
  SafeAreaView,
  StatusBar,
  ActivityIndicator,
  RefreshControl,
} from 'react-native';
import { Svg, Line, Circle, Text as SvgText, Rect, G, Path, Polyline, Defs, LinearGradient, Stop } from 'react-native-svg';

const API_BASE = 'http://localhost:7879/api/v1';

const CHART_COLORS = ['#4e79a7', '#f28e2b', '#e15759', '#76b7b2', '#59a14f', '#edc948', '#b07aa1', '#ff9da7'];

export default function App() {
  const [activeTab, setActiveTab] = useState('query');
  const [sql, setSql] = useState('SELECT * FROM cpu');
  const [queryResult, setQueryResult] = useState(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);
  const [writeForm, setWriteForm] = useState({
    measurement: 'cpu',
    tags: 'host=server01,region=us-west',
    fields: 'usage=0.85,idle=0.15',
  });
  const [chartData, setChartData] = useState(null);
  const [dashboardData, setDashboardData] = useState(null);
  const [refreshing, setRefreshing] = useState(false);
  const [timeRange, setTimeRange] = useState('1h');
  const [multiSeriesData, setMultiSeriesData] = useState(null);

  const executeQuery = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(`${API_BASE}/query`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sql }),
      });
      const data = await response.json();
      if (data.error) {
        setError(data.error);
      } else {
        setQueryResult(data);
      }
    } catch (e) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }, [sql]);

  const writeData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const tags = {};
      writeForm.tags.split(',').forEach(pair => {
        const [k, v] = pair.split('=');
        if (k && v) tags[k.trim()] = v.trim();
      });

      const fields = {};
      writeForm.fields.split(',').forEach(pair => {
        const [k, v] = pair.split('=');
        if (k && v) {
          const num = parseFloat(v);
          fields[k.trim()] = isNaN(num) ? v.trim() : num;
        }
      });

      const response = await fetch(`${API_BASE}/write`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          measurement: writeForm.measurement,
          tags,
          fields,
          timestamp: Date.now() * 1000,
        }),
      });
      if (response.ok) {
        setError(null);
        setQueryResult({ message: 'Write successful' });
      } else {
        const data = await response.json();
        setError(data.error || 'Write failed');
      }
    } catch (e) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }, [writeForm]);

  const generateChart = useCallback(async () => {
    setLoading(true);
    try {
      const response = await fetch(`${API_BASE}/chart`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sql, chart_type: 'line', title: 'Time Series' }),
      });
      if (response.ok) {
        const svgText = await response.text();
        setChartData(svgText);
      }
    } catch (e) {
      setError(e.message);
    } finally {
      setLoading(false);
    }
  }, [sql]);

  const pingServer = useCallback(async () => {
    try {
      const response = await fetch(`${API_BASE}/ping`);
      const text = await response.text();
      setQueryResult({ message: text });
    } catch (e) {
      setError('Server not reachable: ' + e.message);
    }
  }, []);

  const fetchDashboardData = useCallback(async () => {
    try {
      const [cpuRes, memRes, diskRes] = await Promise.all([
        fetch(`${API_BASE}/query`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ sql: 'SELECT AVG(usage_user) FROM cpu' }),
        }).catch(() => null),
        fetch(`${API_BASE}/query`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ sql: 'SELECT AVG(usage_user) FROM cpu WHERE hostname=host1' }),
        }).catch(() => null),
        fetch(`${API_BASE}/databases`, {
          method: 'GET',
        }).catch(() => null),
      ]);

      const cpuData = cpuRes ? await cpuRes.json().catch(() => null) : null;
      const memData = memRes ? await memRes.json().catch(() => null) : null;
      const diskData = diskRes ? await diskRes.json().catch(() => null) : null;

      setDashboardData({
        cpu: cpuData?.rows?.[0]?.[0] ?? 0,
        memory: memData?.rows?.[0]?.[0] ?? 0,
        disk: diskData?.rows?.[0]?.[0] ?? 0,
        databases: diskData?.databases || ['default'],
        uptime: new Date().toLocaleTimeString(),
      });
    } catch (e) {
      setDashboardData({
        cpu: 0,
        memory: 0,
        disk: 0,
        databases: ['default'],
        uptime: new Date().toLocaleTimeString(),
      });
    }
  }, []);

  const fetchMultiSeries = useCallback(async () => {
    try {
      const response = await fetch(`${API_BASE}/query`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sql }),
      });
      const data = await response.json();
      if (data.columns && data.rows) {
        setMultiSeriesData(data);
      }
    } catch (e) {
      // ignore
    }
  }, [sql]);

  const onRefresh = useCallback(async () => {
    setRefreshing(true);
    await fetchDashboardData();
    setRefreshing(false);
  }, [fetchDashboardData]);

  useEffect(() => {
    if (activeTab === 'dashboard') {
      fetchDashboardData();
    }
  }, [activeTab, fetchDashboardData]);

  const renderTable = () => {
    if (!queryResult || !queryResult.columns) return null;
    return (
      <ScrollView horizontal style={styles.tableContainer}>
        <View>
          <View style={styles.tableHeader}>
            {queryResult.columns.map((col, i) => (
              <Text key={i} style={styles.tableHeaderCell}>{col}</Text>
            ))}
          </View>
          <FlatList
            data={queryResult.rows}
            keyExtractor={(_, i) => i.toString()}
            renderItem={({ item }) => (
              <View style={styles.tableRow}>
                {item.map((cell, i) => (
                  <Text key={i} style={styles.tableCell}>
                    {typeof cell === 'object' ? JSON.stringify(cell) : String(cell)}
                  </Text>
                ))}
              </View>
            )}
          />
        </View>
      </ScrollView>
    );
  };

  const renderMiniChart = () => {
    if (!queryResult || !queryResult.rows || queryResult.rows.length === 0) return null;
    const timeIdx = queryResult.columns.indexOf('time');
    if (timeIdx === -1) return null;

    const values = queryResult.rows.map(r => {
      for (let i = 0; i < r.length; i++) {
        if (i !== timeIdx && typeof r[i] === 'number') return r[i];
      }
      return 0;
    }).filter(v => v !== 0);

    if (values.length === 0) return null;

    const min = Math.min(...values);
    const max = Math.max(...values);
    const range = max - min || 1;
    const w = 350;
    const h = 150;
    const padding = 20;

    const points = values.map((v, i) => ({
      x: padding + (i / (values.length - 1 || 1)) * (w - 2 * padding),
      y: h - padding - ((v - min) / range) * (h - 2 * padding),
    }));

    const linePath = points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');

    return (
      <View style={styles.chartContainer}>
        <Text style={styles.chartTitle}>Trend</Text>
        <Svg width={w} height={h}>
          <Rect x={padding} y={padding} width={w - 2 * padding} height={h - 2 * padding} fill="#f8f9fa" rx="4" />
          <Line x1={padding} y1={h - padding} x2={w - padding} y2={h - padding} stroke="#dee2e6" />
          {points.map((p, i) => (
            <Circle key={i} cx={p.x} cy={p.y} r="3" fill="#4e79a7" />
          ))}
          <Path d={linePath} fill="none" stroke="#4e79a7" strokeWidth="2" />
          <SvgText x={padding} y={h - 5} fontSize="10" fill="#666">{min.toFixed(1)}</SvgText>
          <SvgText x={padding} y={padding - 5} fontSize="10" fill="#666">{max.toFixed(1)}</SvgText>
        </Svg>
      </View>
    );
  };

  const renderMultiSeriesChart = () => {
    if (!multiSeriesData || !multiSeriesData.rows || multiSeriesData.rows.length === 0) return null;

    const timeIdx = multiSeriesData.columns.indexOf('time');
    if (timeIdx === -1) return null;

    const numericCols = [];
    multiSeriesData.columns.forEach((col, i) => {
      if (i !== timeIdx && multiSeriesData.rows.some(r => typeof r[i] === 'number')) {
        numericCols.push({ name: col, index: i });
      }
    });

    if (numericCols.length === 0) return null;

    const w = 350;
    const h = 200;
    const padding = 30;

    let allValues = [];
    numericCols.forEach(col => {
      multiSeriesData.rows.forEach(r => {
        if (typeof r[col.index] === 'number') allValues.push(r[col.index]);
      });
    });

    const min = Math.min(...allValues);
    const max = Math.max(...allValues);
    const range = max - min || 1;

    return (
      <View style={styles.chartContainer}>
        <Text style={styles.chartTitle}>Multi-Series Comparison</Text>
        <Svg width={w} height={h}>
          <Rect x={padding} y={padding} width={w - 2 * padding} height={h - 2 * padding} fill="#f8f9fa" rx="4" />
          <Line x1={padding} y1={h - padding} x2={w - padding} y2={h - padding} stroke="#dee2e6" />
          <Line x1={padding} y1={padding} x2={padding} y2={h - padding} stroke="#dee2e6" />

          {numericCols.map((col, ci) => {
            const values = multiSeriesData.rows.map(r => r[col.index]).filter(v => typeof v === 'number');
            const points = values.map((v, i) => ({
              x: padding + (i / (values.length - 1 || 1)) * (w - 2 * padding),
              y: h - padding - ((v - min) / range) * (h - 2 * padding),
            }));
            const pathD = points.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x} ${p.y}`).join(' ');
            const color = CHART_COLORS[ci % CHART_COLORS.length];

            return (
              <G key={ci}>
                <Path d={pathD} fill="none" stroke={color} strokeWidth="2" />
                {points.map((p, pi) => (
                  <Circle key={pi} cx={p.x} cy={p.y} r="2" fill={color} />
                ))}
              </G>
            );
          })}

          <SvgText x={padding} y={h - 5} fontSize="9" fill="#666">{min.toFixed(1)}</SvgText>
          <SvgText x={padding} y={padding - 5} fontSize="9" fill="#666">{max.toFixed(1)}</SvgText>

          {numericCols.map((col, ci) => (
            <G key={`legend-${ci}`}>
              <Rect x={padding + ci * 80} y={5} width={10} height={10} fill={CHART_COLORS[ci % CHART_COLORS.length]} />
              <SvgText x={padding + ci * 80 + 14} y={14} fontSize="9" fill="#333">{col.name}</SvgText>
            </G>
          ))}
        </Svg>

        <View style={styles.timeRangeBar}>
          {['5m', '15m', '1h', '6h', '24h'].map(range => (
            <TouchableOpacity
              key={range}
              style={[styles.timeRangeButton, timeRange === range && styles.activeTimeRange]}
              onPress={() => setTimeRange(range)}
            >
              <Text style={[styles.timeRangeText, timeRange === range && styles.activeTimeRangeText]}>
                {range}
              </Text>
            </TouchableOpacity>
          ))}
        </View>
      </View>
    );
  };

  const renderDashboard = () => {
    if (!dashboardData) {
      return (
        <View style={styles.loadingContainer}>
          <ActivityIndicator size="large" color="#4e79a7" />
          <Text style={styles.loadingText}>Loading dashboard...</Text>
        </View>
      );
    }

    const metrics = [
      { label: 'CPU Usage', value: dashboardData.cpu, unit: '%', color: '#4e79a7', icon: '⚡' },
      { label: 'Memory', value: dashboardData.memory, unit: '%', color: '#f28e2b', icon: '💾' },
      { label: 'Disk I/O', value: dashboardData.disk, unit: '%', color: '#59a14f', icon: '💿' },
    ];

    return (
      <ScrollView
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} />}
      >
        <Text style={styles.sectionTitle}>System Overview</Text>
        <View style={styles.metricsGrid}>
          {metrics.map((metric, i) => (
            <View key={i} style={[styles.metricCard, { borderLeftColor: metric.color }]}>
              <Text style={styles.metricIcon}>{metric.icon}</Text>
              <Text style={styles.metricLabel}>{metric.label}</Text>
              <View style={styles.metricValueRow}>
                <Text style={[styles.metricValue, { color: metric.color }]}>
                  {typeof metric.value === 'number' ? metric.value.toFixed(1) : metric.value}
                </Text>
                <Text style={styles.metricUnit}>{metric.unit}</Text>
              </View>
              <View style={styles.progressBar}>
                <View style={[styles.progressFill, {
                  width: `${Math.min(typeof metric.value === 'number' ? metric.value : 0, 100)}%`,
                  backgroundColor: metric.color,
                }]} />
              </View>
            </View>
          ))}
        </View>

        <Text style={styles.sectionTitle}>Databases</Text>
        <View style={styles.dbList}>
          {dashboardData.databases.map((db, i) => (
            <View key={i} style={styles.dbCard}>
              <Text style={styles.dbName}>{db}</Text>
              <Text style={styles.dbStatus}>Active</Text>
            </View>
          ))}
        </View>

        <Text style={styles.sectionTitle}>Server Info</Text>
        <View style={styles.infoCard}>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>API Endpoint</Text>
            <Text style={styles.infoValue}>{API_BASE}</Text>
          </View>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>Last Updated</Text>
            <Text style={styles.infoValue}>{dashboardData.uptime}</Text>
          </View>
          <View style={styles.infoRow}>
            <Text style={styles.infoLabel}>DB Count</Text>
            <Text style={styles.infoValue}>{dashboardData.databases.length}</Text>
          </View>
        </View>
      </ScrollView>
    );
  };

  const tabs = ['query', 'write', 'chart', 'dashboard', 'status'];

  return (
    <SafeAreaView style={styles.container}>
      <StatusBar barStyle="dark-content" />
      <View style={styles.header}>
        <Text style={styles.headerTitle}>TSDB Manager</Text>
      </View>

      <View style={styles.tabBar}>
        {tabs.map(tab => (
          <TouchableOpacity
            key={tab}
            style={[styles.tab, activeTab === tab && styles.activeTab]}
            onPress={() => setActiveTab(tab)}
          >
            <Text style={[styles.tabText, activeTab === tab && styles.activeTabText]}>
              {tab.charAt(0).toUpperCase() + tab.slice(1)}
            </Text>
          </TouchableOpacity>
        ))}
      </View>

      <ScrollView style={styles.content}>
        {activeTab === 'query' && (
          <View>
            <TextInput
              style={styles.sqlInput}
              multiline
              value={sql}
              onChangeText={setSql}
              placeholder="Enter SQL query"
            />
            <TouchableOpacity style={styles.button} onPress={executeQuery}>
              <Text style={styles.buttonText}>Execute Query</Text>
            </TouchableOpacity>
            {loading && <ActivityIndicator size="large" color="#4e79a7" />}
            {error && <Text style={styles.errorText}>Error: {error}</Text>}
            {renderMiniChart()}
            {renderTable()}
          </View>
        )}

        {activeTab === 'write' && (
          <View>
            <Text style={styles.label}>Measurement</Text>
            <TextInput
              style={styles.input}
              value={writeForm.measurement}
              onChangeText={v => setWriteForm({ ...writeForm, measurement: v })}
            />
            <Text style={styles.label}>Tags (key=value, separated by commas)</Text>
            <TextInput
              style={styles.input}
              value={writeForm.tags}
              onChangeText={v => setWriteForm({ ...writeForm, tags: v })}
            />
            <Text style={styles.label}>Fields (key=value, separated by commas)</Text>
            <TextInput
              style={styles.input}
              value={writeForm.fields}
              onChangeText={v => setWriteForm({ ...writeForm, fields: v })}
            />
            <TouchableOpacity style={styles.button} onPress={writeData}>
              <Text style={styles.buttonText}>Write Data</Text>
            </TouchableOpacity>
            {loading && <ActivityIndicator size="large" color="#4e79a7" />}
            {error && <Text style={styles.errorText}>Error: {error}</Text>}
          </View>
        )}

        {activeTab === 'chart' && (
          <View>
            <TextInput
              style={styles.sqlInput}
              multiline
              value={sql}
              onChangeText={setSql}
              placeholder="SQL for chart data"
            />
            <View style={styles.buttonRow}>
              <TouchableOpacity style={[styles.button, styles.buttonHalf]} onPress={generateChart}>
                <Text style={styles.buttonText}>SVG Chart</Text>
              </TouchableOpacity>
              <TouchableOpacity style={[styles.button, styles.buttonHalf]} onPress={fetchMultiSeries}>
                <Text style={styles.buttonText}>Multi-Series</Text>
              </TouchableOpacity>
            </View>
            {loading && <ActivityIndicator size="large" color="#4e79a7" />}
            {renderMiniChart()}
            {renderMultiSeriesChart()}
          </View>
        )}

        {activeTab === 'dashboard' && renderDashboard()}

        {activeTab === 'status' && (
          <View>
            <TouchableOpacity style={styles.button} onPress={pingServer}>
              <Text style={styles.buttonText}>Ping Server</Text>
            </TouchableOpacity>
            {queryResult && queryResult.message && (
              <View style={styles.statusCard}>
                <Text style={styles.statusText}>Server: {queryResult.message}</Text>
              </View>
            )}
            <View style={styles.statusCard}>
              <Text style={styles.statusText}>API: {API_BASE}</Text>
            </View>
          </View>
        )}
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: '#f5f5f5' },
  header: { backgroundColor: '#4e79a7', padding: 16, alignItems: 'center' },
  headerTitle: { color: 'white', fontSize: 20, fontWeight: 'bold' },
  tabBar: { flexDirection: 'row', backgroundColor: 'white', borderBottomWidth: 1, borderBottomColor: '#e0e0e0' },
  tab: { flex: 1, paddingVertical: 12, alignItems: 'center' },
  activeTab: { borderBottomWidth: 2, borderBottomColor: '#4e79a7' },
  tabText: { fontSize: 12, color: '#666' },
  activeTabText: { color: '#4e79a7', fontWeight: 'bold' },
  content: { flex: 1, padding: 16 },
  sqlInput: { backgroundColor: 'white', borderRadius: 8, padding: 12, fontSize: 14, minHeight: 80, borderWidth: 1, borderColor: '#ddd' },
  input: { backgroundColor: 'white', borderRadius: 8, padding: 12, fontSize: 14, borderWidth: 1, borderColor: '#ddd', marginBottom: 8 },
  label: { fontSize: 14, fontWeight: '600', marginBottom: 4, color: '#333' },
  button: { backgroundColor: '#4e79a7', borderRadius: 8, padding: 14, alignItems: 'center', marginTop: 12 },
  buttonRow: { flexDirection: 'row', gap: 8, marginTop: 12 },
  buttonHalf: { flex: 1, marginTop: 0 },
  buttonText: { color: 'white', fontSize: 16, fontWeight: 'bold' },
  errorText: { color: '#e15759', marginTop: 8, fontSize: 14 },
  tableContainer: { marginTop: 16 },
  tableHeader: { flexDirection: 'row', backgroundColor: '#4e79a7', borderTopLeftRadius: 8, borderTopRightRadius: 8 },
  tableHeaderCell: { color: 'white', padding: 8, fontSize: 12, fontWeight: 'bold', minWidth: 100 },
  tableRow: { flexDirection: 'row', borderBottomWidth: 1, borderBottomColor: '#eee', backgroundColor: 'white' },
  tableCell: { padding: 8, fontSize: 12, minWidth: 100, color: '#333' },
  chartContainer: { marginTop: 16, backgroundColor: 'white', borderRadius: 8, padding: 12, alignItems: 'center' },
  chartTitle: { fontSize: 16, fontWeight: 'bold', marginBottom: 8, color: '#333' },
  statusCard: { backgroundColor: 'white', borderRadius: 8, padding: 16, marginTop: 12 },
  statusText: { fontSize: 14, color: '#333' },
  loadingContainer: { alignItems: 'center', justifyContent: 'center', padding: 40 },
  loadingText: { marginTop: 12, color: '#666', fontSize: 14 },
  sectionTitle: { fontSize: 18, fontWeight: 'bold', color: '#333', marginTop: 16, marginBottom: 8 },
  metricsGrid: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  metricCard: { backgroundColor: 'white', borderRadius: 8, padding: 12, width: '31%', borderLeftWidth: 4 },
  metricIcon: { fontSize: 20, marginBottom: 4 },
  metricLabel: { fontSize: 12, color: '#666', marginBottom: 4 },
  metricValueRow: { flexDirection: 'row', alignItems: 'baseline' },
  metricValue: { fontSize: 24, fontWeight: 'bold' },
  metricUnit: { fontSize: 12, color: '#999', marginLeft: 2 },
  progressBar: { height: 4, backgroundColor: '#eee', borderRadius: 2, marginTop: 8 },
  progressFill: { height: '100%', borderRadius: 2 },
  dbList: { gap: 8 },
  dbCard: { backgroundColor: 'white', borderRadius: 8, padding: 12, flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center' },
  dbName: { fontSize: 14, fontWeight: '600', color: '#333' },
  dbStatus: { fontSize: 12, color: '#59a14f', fontWeight: '600' },
  infoCard: { backgroundColor: 'white', borderRadius: 8, padding: 12 },
  infoRow: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: 6, borderBottomWidth: 1, borderBottomColor: '#f0f0f0' },
  infoLabel: { fontSize: 13, color: '#666' },
  infoValue: { fontSize: 13, color: '#333', fontWeight: '500' },
  timeRangeBar: { flexDirection: 'row', marginTop: 12, gap: 4, justifyContent: 'center' },
  timeRangeButton: { paddingHorizontal: 12, paddingVertical: 6, borderRadius: 12, backgroundColor: '#f0f0f0' },
  activeTimeRange: { backgroundColor: '#4e79a7' },
  timeRangeText: { fontSize: 12, color: '#666' },
  activeTimeRangeText: { color: 'white' },
});
