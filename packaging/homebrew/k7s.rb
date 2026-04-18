# Homebrew formula for k7s — Phase 14.8
#
# To install from a local tap:
#   brew tap your-org/k7s https://github.com/your-org/homebrew-k7s
#   brew install k7s
#
# Or to install directly from a local checkout (for development):
#   brew install --build-from-source ./packaging/homebrew/k7s.rb
#
# Homebrew formula documentation:
#   https://docs.brew.sh/Formula-Cookbook

class K7s < Formula
  desc "Performance-focused, security-first Kubernetes TUI with AI-powered cluster analysis"
  homepage "https://github.com/your-org/k7s"
  url "https://github.com/your-org/k7s/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_WITH_ACTUAL_SHA256_OF_RELEASE_TARBALL"
  license "Apache-2.0"
  head "https://github.com/your-org/k7s.git", branch: "main"

  # Build-time dependencies.
  depends_on "rust" => :build

  # Runtime: kubectl must be installed for CLI parity features (exec, logs, etc.)
  depends_on "kubernetes-cli" => :recommended

  # Optional: trivy for image vulnerability scanning (:vuln view).
  depends_on "aquasecurity/trivy/trivy" => :optional

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      k7s requires a valid kubeconfig to connect to a Kubernetes cluster.
      The default location is ~/.kube/config (KUBECONFIG env var is supported).

      AI chat features require an API key:
        export K7S_LLM_API_KEY=<your-key>
      or add it to ~/.config/k7s/config.yaml:
        k7s:
          ai:
            apiKey: <your-key>

      For Google Antigravity (ADC) authentication:
        gcloud auth application-default login

      To enable image vulnerability scanning install trivy:
        brew install aquasecurity/trivy/trivy
    EOS
  end

  test do
    # Smoke-test: binary exists and prints version without connecting to a cluster.
    assert_match "k7s", shell_output("#{bin}/k7s version 2>&1")
  end
end
