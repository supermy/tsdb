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
} from 'react-native';
import { Svg, Line, Circle, Text as SvgText, Rect, G } from 'react-native-svg';

const API_BASE = 'http://localhost:7879/api/v1';

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
          <G>
            {linePath && <Line x1={0} y1={0} x2={0} y2={0} stroke="#4e79a7" strokeWidth="2" />}
          </G>
          <SvgText x={padding} y={h - 5} fontSize="10" fill="#666">{min.toFixed(1)}</SvgText>
          <SvgText x={padding} y={padding - 5} fontSize="10" fill="#666">{max.toFixed(1)}</SvgText>
        </Svg>
      </View>
    );
  };

  return (
    <SafeAreaView style={styles.container}>
      <StatusBar barStyle="dark-content" />
      <View style={styles.header}>
        <Text style={styles.headerTitle}>TSDB Manager</Text>
      </View>

      <View style={styles.tabBar}>
        {['query', 'write', 'chart', 'status'].map(tab => (
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
            <TouchableOpacity style={styles.button} onPress={generateChart}>
              <Text style={styles.buttonText}>Generate Chart</Text>
            </TouchableOpacity>
            {loading && <ActivityIndicator size="large" color="#4e79a7" />}
            {renderMiniChart()}
          </View>
        )}

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
  tabText: { fontSize: 14, color: '#666' },
  activeTabText: { color: '#4e79a7', fontWeight: 'bold' },
  content: { flex: 1, padding: 16 },
  sqlInput: { backgroundColor: 'white', borderRadius: 8, padding: 12, fontSize: 14, minHeight: 80, borderWidth: 1, borderColor: '#ddd' },
  input: { backgroundColor: 'white', borderRadius: 8, padding: 12, fontSize: 14, borderWidth: 1, borderColor: '#ddd', marginBottom: 8 },
  label: { fontSize: 14, fontWeight: '600', marginBottom: 4, color: '#333' },
  button: { backgroundColor: '#4e79a7', borderRadius: 8, padding: 14, alignItems: 'center', marginTop: 12 },
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
});
