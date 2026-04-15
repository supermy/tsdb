#!/bin/bash
set -e

echo "=== TSDB TSBS Data Generator ==="

SCALE=${1:-4000}
DURATION=${2:-"96h"}
OUTPUT=${3:-"tsbs_data.json"}

echo "Generating DevOps data: scale=$SCALE, duration=$DURATION"

if ! command -v tsbs_generate_data &> /dev/null; then
    echo "Installing TSBS tools..."
    go install github.com/timescale/tsbs/cmd/tsbs_generate_data@latest 2>/dev/null || {
        echo "Warning: tsbs_generate_data not available."
        echo "Generating synthetic data instead..."
        cargo run --release --bin tsdb-cli -- generate-tsbs --scale $SCALE --duration $DURATION --output $OUTPUT
        exit 0
    }
fi

tsbs_generate_data \
    --use-case="devops" \
    --scale=$SCALE \
    --timestamp-start="2025-01-01T00:00:00Z" \
    --timestamp-end="2025-01-05T00:00:00Z" \
    --file-format="json" \
    --seed=42 \
    > $OUTPUT

echo "Generated: $OUTPUT ($(wc -l < $OUTPUT) lines)"
