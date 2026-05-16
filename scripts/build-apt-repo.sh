#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <deb-file> <repo-root> <origin-url>" >&2
  exit 1
fi

deb_file="$1"
repo_root="$2"
origin_url="$3"
package_name="agent-orchestrator"
distribution="stable"
component="main"
arch="amd64"

pool_dir="${repo_root}/pool/${component}/${package_name:0:1}/${package_name}"
dist_dir="${repo_root}/dists/${distribution}/${component}/binary-${arch}"

mkdir -p "$pool_dir" "$dist_dir"
cp "$deb_file" "$pool_dir/"

dpkg-scanpackages --multiversion "$pool_dir" > "${dist_dir}/Packages"
gzip -9 -c "${dist_dir}/Packages" > "${dist_dir}/Packages.gz"

cat > "${repo_root}/apt-ftparchive.conf" <<EOF
APT::FTPArchive::Release {
  Origin "petarnenov";
  Label "agent-orchestrator";
  Suite "${distribution}";
  Codename "${distribution}";
  Architectures "${arch}";
  Components "${component}";
  Description "APT repository for agent-orchestrator";
};
EOF

apt-ftparchive -c "${repo_root}/apt-ftparchive.conf" \
  release "${repo_root}/dists/${distribution}" > "${repo_root}/dists/${distribution}/Release"

key_id="$(gpg --batch --list-secret-keys --with-colons | awk -F: '/^sec:/ {print $5; exit}')"
if [ -z "$key_id" ]; then
  echo "no secret GPG key available for signing" >&2
  exit 1
fi

gpg --batch --yes --armor --output "${repo_root}/dists/${distribution}/Release.gpg" \
  --detach-sign --local-user "$key_id" "${repo_root}/dists/${distribution}/Release"

gpg --batch --yes --clearsign --output "${repo_root}/dists/${distribution}/InRelease" \
  --local-user "$key_id" "${repo_root}/dists/${distribution}/Release"

gpg --batch --yes --armor --export "$key_id" > "${repo_root}/public.key"

cat > "${repo_root}/index.html" <<EOF
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <title>agent-orchestrator APT repository</title>
  </head>
  <body>
    <h1>agent-orchestrator APT repository</h1>
    <p>Repository URL: ${origin_url}</p>
    <p>Public key: <a href="public.key">public.key</a></p>
  </body>
</html>
EOF
