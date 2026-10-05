import { existsSync,mkdirSync,writeFileSync } from "node:fs";
import { join } from "node:path";
import { qaProfile,realLoginFromStdin } from "./options.mjs";

export function childEnvironment(dataDir, qaLoginPipePath = null, qaControlPipePath = null) {
  const allowedKeys = [
    "AR",
    "CARGO_HOME",
    "CARGO_TARGET_DIR",
    "CC",
    "CFLAGS",
    "CPATH",
    "CPPFLAGS",
    "CXX",
    "CXXFLAGS",
    "DBUS_SESSION_BUS_ADDRESS",
    "DISPLAY",
    "GDK_BACKEND",
    "HOME",
    "KOUSHI_CORE_ACTOR_TRACE",
    "LANG",
    "LC_ALL",
    "LDFLAGS",
    "LIBRARY_PATH",
    "LOGNAME",
    "NPM_CONFIG_USERCONFIG",
    "PATH",
    "PKG_CONFIG_PATH",
    "RUSTFLAGS",
    "RUSTUP_HOME",
    "SHELL",
    "TMPDIR",
    "USER",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "npm_config_userconfig"
  ];
  const env = {};
  for (const key of allowedKeys) {
    if (process.env[key]) {
      env[key] = process.env[key];
    }
  }
  env.GDK_BACKEND = "x11";
  env.KOUSHI_RESTORE_SESSION = qaProfile !== undefined ? "1" : "0";
  env.KOUSHI_SKIP_SAVED_SESSIONS = "1";
  env.KOUSHI_SKIP_KEYCHAIN_PERSISTENCE = "1";
  env.KOUSHI_DATA_DIR = dataDir;
  env.KOUSHI_QA_TITLE = "1";
  env.VITE_KOUSHI_QA_TITLE = "1";
  env.KOUSHI_QA_FILE_CREDENTIAL_STORE_DIR = join(dataDir, "qa-credential-store");
  env.NO_COLOR = "1";
  if (qaProfile !== undefined) {
    env.KOUSHI_RESTORE_SESSION = "1";
  }
  if (qaLoginPipePath) {
    env.KOUSHI_QA_LOGIN_PIPE = qaLoginPipePath;
  } else if (realLoginFromStdin) {
    env.KOUSHI_QA_LOGIN_PIPE = join(dataDir, "qa-login.pipe");
  }
  if (qaControlPipePath) {
    env.KOUSHI_QA_CONTROL_PIPE = qaControlPipePath;
  } else if (realLoginFromStdin) {
    env.KOUSHI_QA_CONTROL_PIPE = join(dataDir, "qa-control.pipe");
  }
  Object.assign(env, nssWrapperEnvironment(dataDir));
  return env;
}


function nssWrapperEnvironment(dataDir) {
  // The image is built for either architecture; pick whichever library this
  // container actually ships instead of assuming the x86_64 path.
  const libraryPath = [
    "/usr/lib/x86_64-linux-gnu/libnss_wrapper.so",
    "/usr/lib/aarch64-linux-gnu/libnss_wrapper.so"
  ].find((candidate) => existsSync(candidate));
  if (!libraryPath) {
    return {};
  }

  const uid = typeof process.getuid === "function" ? process.getuid() : null;
  const gid = typeof process.getgid === "function" ? process.getgid() : null;
  if (!Number.isInteger(uid) || !Number.isInteger(gid)) {
    return {};
  }

  const nssDir = join(dataDir, "qa-nss-wrapper");
  mkdirSync(nssDir, { recursive: true });

  const passwdPath = join(nssDir, "passwd");
  const groupPath = join(nssDir, "group");
  writeFileSync(passwdPath, `koushi-desktop:x:${uid}:${gid}:Koushi:/tmp:/bin/sh\n`);
  writeFileSync(groupPath, `koushi-desktop:x:${gid}:\n`);

  return {
    LD_PRELOAD: buildLdPreload(libraryPath),
    NSS_WRAPPER_PASSWD: passwdPath,
    NSS_WRAPPER_GROUP: groupPath
  };
}


function buildLdPreload(libraryPath) {
  const existing = process.env.LD_PRELOAD?.trim();
  return existing ? `${libraryPath} ${existing}` : libraryPath;
}
