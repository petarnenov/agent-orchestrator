#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 8 ]; then
  echo "usage: $0 <version> <owner> <repo> <intel-url> <intel-sha> <arm-url> <arm-sha> <output-file>" >&2
  exit 1
fi

version="$1"
owner="$2"
repo="$3"
intel_url="$4"
intel_sha="$5"
arm_url="$6"
arm_sha="$7"
output_file="$8"

mkdir -p "$(dirname "$output_file")"

cat > "$output_file" <<EOF
class AgentOrchestrator < Formula
  desc "CLI orchestrator for Copilot and Claude execution pipelines"
  homepage "https://github.com/${owner}/${repo}"
  version "${version}"
  license "MIT"

  on_macos do
    on_intel do
      url "${intel_url}"
      sha256 "${intel_sha}"
    end

    on_arm do
      url "${arm_url}"
      sha256 "${arm_sha}"
    end
  end

  def install
    bin.install "agent-orchestrator"
  end

  test do
    assert_match "CLI orchestrator", shell_output("#{bin}/agent-orchestrator --help")
  end
end
EOF
