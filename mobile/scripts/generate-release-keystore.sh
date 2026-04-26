#!/usr/bin/env bash
# generate-release-keystore.sh
#
# === RUN THIS ONCE ON THE BUILD MACHINE. ===
# === COPY THE KEYSTORE + PASSWORD FILE TO A SAFE BACKUP LOCATION ===
# === BEFORE DOING ANYTHING ELSE WITH THE GENERATED MATERIAL.    ===
#
# Generates a persistent JKS *production* upload-keystore for PhantomChat's
# Android release builds. The keystore lives at:
#
#     ~/.android/phantomchat-release.jks
#
# and the matching CSPRNG-generated 32-character password lives at:
#
#     ~/.android/phantomchat-release.password.txt        (mode 0600)
#
# Wire it into the build via mobile/android/key.properties — see the template
# at mobile/android/key.properties.template.
#
# IMPORTANT: Losing the keystore *or* the password means PhantomChat can never
# again ship updates to existing installs (Android refuses upgrades whose
# signature does not match the originally installed signature).
#
# Distinguished name defaults to:
#   CN=PhantomChat, OU=DC INFOSEC, O=DC INFOSEC, L=Berlin, ST=Berlin, C=DE
# Override the city via:
#   --cn-city <city>      (CLI flag)
#   PHANTOMCHAT_CN_CITY   (env var)

set -euo pipefail

KEYSTORE_PATH="${HOME}/.android/phantomchat-release.jks"
PASSWORD_PATH="${HOME}/.android/phantomchat-release.password.txt"
ALIAS="phantomchat"
KEYALG="RSA"
KEYSIZE="4096"
VALIDITY_DAYS="10000"   # ~27 years; Play Store recommends >=25

CN_CITY_DEFAULT="Berlin"
CN_CITY="${PHANTOMCHAT_CN_CITY:-$CN_CITY_DEFAULT}"

# --- arg parsing ---------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --cn-city)
            CN_CITY="${2:?--cn-city requires a value}"
            shift 2
            ;;
        -h|--help)
            sed -n '2,30p' "$0"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

DNAME="CN=PhantomChat, OU=DC INFOSEC, O=DC INFOSEC, L=${CN_CITY}, ST=${CN_CITY}, C=DE"

# --- preflight ----------------------------------------------------------
if ! command -v keytool >/dev/null 2>&1; then
    echo "ERROR: 'keytool' not found in PATH." >&2
    echo "Install a Java JDK (e.g. 'sudo apt install default-jdk' or use the JDK bundled with Android Studio)." >&2
    exit 1
fi

mkdir -p "$(dirname "$KEYSTORE_PATH")"

# --- idempotency --------------------------------------------------------
if [[ -f "$KEYSTORE_PATH" ]]; then
    echo "INFO: keystore already exists at: $KEYSTORE_PATH"
    echo "INFO: refusing to overwrite. Delete it manually if you really want a fresh one"
    echo "INFO: (and remember: the OLD one is the only thing that can sign updates for existing installs)."
    exit 0
fi

# --- password generation (CSPRNG, 32 chars, alnum) ----------------------
# tr-strip to alnum so passwords copy/paste cleanly into Gradle without escaping.
PASSWORD="$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 32)"
if [[ ${#PASSWORD} -ne 32 ]]; then
    echo "ERROR: failed to generate 32-char password from /dev/urandom" >&2
    exit 1
fi

umask 077
printf '%s\n' "$PASSWORD" > "$PASSWORD_PATH"
chmod 600 "$PASSWORD_PATH"

# --- keystore generation ------------------------------------------------
echo "Generating PhantomChat production upload-keystore..."
echo "  path:       $KEYSTORE_PATH"
echo "  alias:      $ALIAS"
echo "  algorithm:  $KEYALG $KEYSIZE"
echo "  validity:   $VALIDITY_DAYS days"
echo "  dname:      $DNAME"
echo

keytool -genkey -v \
    -alias "$ALIAS" \
    -keyalg "$KEYALG" \
    -keysize "$KEYSIZE" \
    -validity "$VALIDITY_DAYS" \
    -keystore "$KEYSTORE_PATH" \
    -storetype JKS \
    -storepass "$PASSWORD" \
    -keypass "$PASSWORD" \
    -dname "$DNAME"

chmod 600 "$KEYSTORE_PATH"

# --- loud warning -------------------------------------------------------
cat <<'WARNING'

################################################################################
#                                                                              #
#   !!!  READ THIS NOW. DO NOT SKIP. DO NOT CLOSE THIS TERMINAL YET.  !!!      #
#                                                                              #
#   A new PhantomChat *production* Android upload-keystore has been created.   #
#                                                                              #
#       Keystore:  ~/.android/phantomchat-release.jks                          #
#       Password:  ~/.android/phantomchat-release.password.txt   (mode 0600)   #
#                                                                              #
#   THIS KEYSTORE IS THE ONLY THING ON EARTH THAT CAN SIGN UPDATES FOR THIS    #
#   APP. IF YOU LOSE IT, OR YOU LOSE THE PASSWORD, THEN:                       #
#                                                                              #
#     * Every existing PhantomChat install in the world becomes               #
#       PERMANENTLY UNUPGRADEABLE. Users will have to uninstall + reinstall.   #
#     * The Play Store listing becomes effectively orphaned -- you cannot      #
#       publish a new version under the same package id ever again.            #
#     * No recovery exists. Not from us, not from Google, not from anyone.     #
#                                                                              #
#   ACTION REQUIRED RIGHT NOW (do all of these):                               #
#                                                                              #
#     1. Copy BOTH files to your password manager (1Password / Bitwarden /     #
#        KeePassXC) as a secure attachment + secure note.                      #
#     2. Copy BOTH files to the Hostinger VPS (which already holds the         #
#        Tauri Updater key) under e.g. /root/secrets/phantomchat/android/      #
#        with root-only permissions.                                           #
#     3. Copy BOTH files to an offline encrypted USB stick stored physically   #
#        separately from your laptop.                                          #
#     4. Verify all three backups by listing them and checking sha256sum       #
#        matches the local copy.                                               #
#                                                                              #
#   TO USE THE KEYSTORE FOR A RELEASE BUILD:                                   #
#                                                                              #
#     a) cp mobile/android/key.properties.template mobile/android/key.properties
#        Then fill in the password from ~/.android/phantomchat-release.password.txt
#        (key.properties is gitignored.)                                       #
#                                                                              #
#     -- OR --                                                                 #
#                                                                              #
#     b) export MYAPP_UPLOAD_STORE_PASSWORD="$(cat ~/.android/phantomchat-release.password.txt)"
#        before running flutter build apk --release.                           #
#                                                                              #
################################################################################

WARNING

echo "Keystore generation complete."
echo "Now go set up backups. Seriously. Do it right now."
