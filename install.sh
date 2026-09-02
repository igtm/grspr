#!/bin/sh

set -eu

owner="igtm"
repo="grspr"
exe_name="grspr"
github_url="https://github.com"
api_url="https://api.github.com"
version=""
executable_folder="${HOME}/.local/bin"
use_gh=false

if command -v gh >/dev/null 2>&1 && command gh auth status --hostname github.com >/dev/null 2>&1; then
    use_gh=true
fi

get_arch() {
    case "$(uname -m)" in
        x86_64|amd64) echo "x86_64" ;;
        aarch64|arm64) echo "aarch64" ;;
        *) echo "" ;;
    esac
}

get_os() {
    case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
        darwin) echo "apple-darwin" ;;
        linux) echo "unknown-linux-gnu" ;;
        *) echo "" ;;
    esac
}

for arg in "$@"; do
    case "${arg}" in
        -b=*) executable_folder="${arg#*=}" ;;
        -v=*) version="${arg#*=}" ;;
        -h|--help)
            echo "usage: install.sh [-b=INSTALL_DIR] [-v=VERSION]"
            exit 0
            ;;
        *)
            echo "ERROR: Unknown argument ${arg}" >&2
            exit 2
            ;;
    esac
done

arch="$(get_arch)"
os="$(get_os)"
if [ -z "${arch}" ] || [ -z "${os}" ]; then
    echo "ERROR: Unsupported platform $(uname -s)/$(uname -m)" >&2
    exit 1
fi

if [ -z "${version}" ]; then
    if [ "${use_gh}" = true ]; then
        version="$(command gh release view --repo "${owner}/${repo}" --json tagName --jq .tagName)"
    else
        version="$(
            command curl -fsSL "${api_url}/repos/${owner}/${repo}/releases/latest" |
            command sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
            command head -n 1
        )"
    fi
fi
if [ -z "${version}" ]; then
    echo "ERROR: Failed to resolve latest release version" >&2
    exit 1
fi
case "${version}" in v*) ;; *) version="v${version}" ;; esac

target="${arch}-${os}"
archive_name="${exe_name}_${version}_${target}.tar.gz"
download_dir="$(mktemp -d)"
trap 'rm -rf "${download_dir}"' EXIT HUP INT TERM
archive_path="${download_dir}/${archive_name}"
checksum_path="${archive_path}.sha256"
asset_url="${github_url}/${owner}/${repo}/releases/download/${version}/${archive_name}"

echo "[1/4] Download ${asset_url}"
if [ "${use_gh}" = true ]; then
    command gh release download "${version}" \
        --repo "${owner}/${repo}" \
        --pattern "${archive_name}" \
        --pattern "${archive_name}.sha256" \
        --dir "${download_dir}"
else
    command curl --fail --location --output "${archive_path}" "${asset_url}"
    command curl --fail --location --output "${checksum_path}" "${asset_url}.sha256"
fi

echo "[2/4] Verify checksum"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "${download_dir}" && command sha256sum -c "${archive_name}.sha256")
elif command -v shasum >/dev/null 2>&1; then
    expected="$(command awk '{print $1}' "${checksum_path}")"
    actual="$(command shasum -a 256 "${archive_path}" | command awk '{print $1}')"
    [ "${expected}" = "${actual}" ] || { echo "ERROR: Checksum mismatch" >&2; exit 1; }
else
    echo "ERROR: sha256sum or shasum is required" >&2
    exit 1
fi

echo "[3/4] Install ${exe_name} to ${executable_folder}"
command mkdir -p "${executable_folder}"
command tar -xzf "${archive_path}" -C "${executable_folder}"
command chmod +x "${executable_folder}/${exe_name}"

echo "[4/4] Done"
echo "${exe_name} was installed to ${executable_folder}/${exe_name}"
