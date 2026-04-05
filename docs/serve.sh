#!/bin/bash
# Serve the datalink documentation website locally

cd "$(dirname "$0")"
echo "Building documentation..."
mdbook build

echo ""
echo "Starting documentation server..."
echo "Open http://localhost:3000 in your browser"
echo "Press Ctrl+C to stop"
echo ""

mdbook serve --open
