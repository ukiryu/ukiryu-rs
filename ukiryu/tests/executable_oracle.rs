//! Golden data captured from the pinned Ruby oracle; do not edit by hand.
//! Oracle: ukiryu/ukiryu @ ab6df353c6dc2d7faf7817a86aafcafa82579896 (Ruby 4.0.1, host linux).
//! Sources: lib/ukiryu/executable_locator.rb, lib/ukiryu/platform.rb, lib/ukiryu/models/executable_info.rb, lib/ukiryu/tool_finder.rb, lib/ukiryu/tool/executable_discovery.rb.
//! Capture script: handoffs/batch-02/K18-DISCOVERY/capture/capture_discovery.rb (raw output sha256 29326e49fa846134ec54b470d82a0f55734e42f96a1c82f45312defc3a0d0f3f).
#![allow(dead_code)]

pub const ORACLE_JSON: &str = r#"{
 "provenance": {
  "oracle": "ukiryu/ukiryu",
  "sha": "ab6df353c6dc2d7faf7817a86aafcafa82579896",
  "ruby": "4.0.1",
  "host_os": "linux",
  "files": [
   "lib/ukiryu/executable_locator.rb",
   "lib/ukiryu/platform.rb",
   "lib/ukiryu/models/executable_info.rb",
   "lib/ukiryu/tool_finder.rb",
   "lib/ukiryu/tool/executable_discovery.rb"
  ],
  "script": "handoffs/batch-02/K18-DISCOVERY/capture/capture_discovery.rb",
  "stubs": "Runtime.platform, Shell.detect/class_for(detect_alias), Executor.execute are recording stubs; Platform.windows? is overridden for simulated-windows candidate cases; PathScanner.executable? is overridden (always false) for candidate-order cases"
 },
 "file_join": [
  {
   "dir": "",
   "name": "x",
   "joined": "/x"
  },
  {
   "dir": "a",
   "name": "x",
   "joined": "a/x"
  },
  {
   "dir": "a/",
   "name": "x",
   "joined": "a/x"
  },
  {
   "dir": "a//",
   "name": "x",
   "joined": "a//x"
  },
  {
   "dir": "a",
   "name": "/x",
   "joined": "a/x"
  },
  {
   "dir": "a/",
   "name": "/x",
   "joined": "a/x"
  },
  {
   "dir": "a//",
   "name": "//x",
   "joined": "a//x"
  },
  {
   "dir": "/",
   "name": "x",
   "joined": "/x"
  },
  {
   "dir": "/",
   "name": "/x",
   "joined": "/x"
  },
  {
   "dir": "//",
   "name": "x",
   "joined": "//x"
  },
  {
   "dir": "a",
   "name": "",
   "joined": "a/"
  },
  {
   "dir": "a/",
   "name": "",
   "joined": "a/"
  },
  {
   "dir": "",
   "name": "",
   "joined": "/"
  },
  {
   "dir": "a",
   "name": "b/c",
   "joined": "a/b/c"
  },
  {
   "dir": "a",
   "name": "x.exe",
   "joined": "a/x.exe"
  },
  {
   "dir": "C:\\bin",
   "name": "x.exe",
   "joined": "C:\\bin/x.exe"
  },
  {
   "dir": "C:/Program Files/X",
   "name": "x.exe",
   "joined": "C:/Program Files/X/x.exe"
  }
 ],
 "path_split": [
  {
   "windows": false,
   "path": null,
   "dirs": []
  },
  {
   "windows": false,
   "path": "",
   "dirs": []
  },
  {
   "windows": false,
   "path": "a",
   "dirs": [
    "a"
   ]
  },
  {
   "windows": false,
   "path": "a:b",
   "dirs": [
    "a",
    "b"
   ]
  },
  {
   "windows": false,
   "path": "a::b",
   "dirs": [
    "a",
    "",
    "b"
   ]
  },
  {
   "windows": false,
   "path": "a:b:",
   "dirs": [
    "a",
    "b"
   ]
  },
  {
   "windows": false,
   "path": "a:b::",
   "dirs": [
    "a",
    "b"
   ]
  },
  {
   "windows": false,
   "path": ":a",
   "dirs": [
    "",
    "a"
   ]
  },
  {
   "windows": false,
   "path": "a;b",
   "dirs": [
    "a;b"
   ]
  },
  {
   "windows": false,
   "path": "a;b;",
   "dirs": [
    "a;b;"
   ]
  },
  {
   "windows": false,
   "path": ";a;;b",
   "dirs": [
    ";a;;b"
   ]
  },
  {
   "windows": false,
   "path": "C:\\x;D:\\y",
   "dirs": [
    "C",
    "\\x;D",
    "\\y"
   ]
  },
  {
   "windows": false,
   "path": "a:b:a",
   "dirs": [
    "a",
    "b",
    "a"
   ]
  },
  {
   "windows": true,
   "path": null,
   "dirs": []
  },
  {
   "windows": true,
   "path": "",
   "dirs": []
  },
  {
   "windows": true,
   "path": "a",
   "dirs": [
    "a"
   ]
  },
  {
   "windows": true,
   "path": "a:b",
   "dirs": [
    "a:b"
   ]
  },
  {
   "windows": true,
   "path": "a::b",
   "dirs": [
    "a::b"
   ]
  },
  {
   "windows": true,
   "path": "a:b:",
   "dirs": [
    "a:b:"
   ]
  },
  {
   "windows": true,
   "path": "a:b::",
   "dirs": [
    "a:b::"
   ]
  },
  {
   "windows": true,
   "path": ":a",
   "dirs": [
    ":a"
   ]
  },
  {
   "windows": true,
   "path": "a;b",
   "dirs": [
    "a",
    "b"
   ]
  },
  {
   "windows": true,
   "path": "a;b;",
   "dirs": [
    "a",
    "b"
   ]
  },
  {
   "windows": true,
   "path": ";a;;b",
   "dirs": [
    "",
    "a",
    "",
    "b"
   ]
  },
  {
   "windows": true,
   "path": "C:\\x;D:\\y",
   "dirs": [
    "C:\\x",
    "D:\\y"
   ]
  },
  {
   "windows": true,
   "path": "a:b:a",
   "dirs": [
    "a:b:a"
   ]
  }
 ],
 "path_extensions": [
  {
   "windows": false,
   "pathext": null,
   "extensions": [
    ""
   ]
  },
  {
   "windows": false,
   "pathext": "",
   "extensions": []
  },
  {
   "windows": false,
   "pathext": ".EXE",
   "extensions": [
    ".EXE"
   ]
  },
  {
   "windows": false,
   "pathext": ".COM;.EXE;.BAT",
   "extensions": [
    ".COM",
    ".EXE",
    ".BAT"
   ]
  },
  {
   "windows": false,
   "pathext": ".com;.exe",
   "extensions": [
    ".com",
    ".exe"
   ]
  },
  {
   "windows": false,
   "pathext": ".BAT;.CMD;.exe;.EXE;.COM",
   "extensions": [
    ".BAT",
    ".CMD",
    ".exe",
    ".EXE",
    ".COM"
   ]
  },
  {
   "windows": false,
   "pathext": ";.sh",
   "extensions": [
    "",
    ".sh"
   ]
  },
  {
   "windows": false,
   "pathext": ".sh;",
   "extensions": [
    ".sh"
   ]
  },
  {
   "windows": false,
   "pathext": ".COM;.EXE;.BAT;.CMD;.VBS",
   "extensions": [
    ".COM",
    ".EXE",
    ".BAT",
    ".CMD",
    ".VBS"
   ]
  },
  {
   "windows": true,
   "pathext": null,
   "extensions": [
    ""
   ]
  },
  {
   "windows": true,
   "pathext": "",
   "extensions": []
  },
  {
   "windows": true,
   "pathext": ".EXE",
   "extensions": [
    ".EXE"
   ]
  },
  {
   "windows": true,
   "pathext": ".COM;.EXE;.BAT",
   "extensions": [
    ".EXE",
    ".COM",
    ".BAT"
   ]
  },
  {
   "windows": true,
   "pathext": ".com;.exe",
   "extensions": [
    ".exe",
    ".com"
   ]
  },
  {
   "windows": true,
   "pathext": ".BAT;.CMD;.exe;.EXE;.COM",
   "extensions": [
    ".exe",
    ".EXE",
    ".BAT",
    ".CMD",
    ".COM"
   ]
  },
  {
   "windows": true,
   "pathext": ";.sh",
   "extensions": [
    "",
    ".sh"
   ]
  },
  {
   "windows": true,
   "pathext": ".sh;",
   "extensions": [
    ".sh"
   ]
  },
  {
   "windows": true,
   "pathext": ".COM;.EXE;.BAT;.CMD;.VBS",
   "extensions": [
    ".EXE",
    ".COM",
    ".BAT",
    ".CMD",
    ".VBS"
   ]
  }
 ],
 "candidates": [
  {
   "windows": false,
   "path": "/a:/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "candidates": [
    "/a/tool",
    "/b/tool"
   ]
  },
  {
   "windows": false,
   "path": "/a:/b:/a",
   "pathext": null,
   "command": "tool",
   "additional": [
    "/c",
    "/b"
   ],
   "candidates": [
    "/a/tool",
    "/b/tool",
    "/c/tool"
   ]
  },
  {
   "windows": false,
   "path": "/a::/b/",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "candidates": [
    "/a/tool",
    "/tool",
    "/b/tool"
   ]
  },
  {
   "windows": false,
   "path": "rel:./x",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "candidates": [
    "rel/tool",
    "./x/tool"
   ]
  },
  {
   "windows": false,
   "path": "/a",
   "pathext": ".sh;.EXE",
   "command": "tool",
   "additional": [],
   "candidates": [
    "/a/tool.sh",
    "/a/tool.EXE"
   ]
  },
  {
   "windows": false,
   "path": "/a",
   "pathext": ";.sh",
   "command": "tool",
   "additional": [],
   "candidates": [
    "/a/tool",
    "/a/tool.sh"
   ]
  },
  {
   "windows": false,
   "path": "/a",
   "pathext": null,
   "command": "sub/tool",
   "additional": [],
   "candidates": [
    "/a/sub/tool"
   ]
  },
  {
   "windows": false,
   "path": "",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "candidates": []
  },
  {
   "windows": false,
   "path": null,
   "pathext": null,
   "command": "tool",
   "additional": [
    "/extra"
   ],
   "candidates": [
    "/extra/tool"
   ]
  },
  {
   "windows": true,
   "path": "C:/Windows/System32;C:/Program Files/gs/gs10/bin",
   "pathext": ".COM;.EXE;.BAT;.CMD",
   "command": "gswin64c",
   "additional": [],
   "candidates": [
    "C:\\Windows\\System32\\gswin64c.EXE",
    "C:\\Windows\\System32\\gswin64c.COM",
    "C:\\Windows\\System32\\gswin64c.BAT",
    "C:\\Windows\\System32\\gswin64c.CMD",
    "C:\\Program Files\\gs\\gs10\\bin\\gswin64c.EXE",
    "C:\\Program Files\\gs\\gs10\\bin\\gswin64c.COM",
    "C:\\Program Files\\gs\\gs10\\bin\\gswin64c.BAT",
    "C:\\Program Files\\gs\\gs10\\bin\\gswin64c.CMD"
   ]
  },
  {
   "windows": true,
   "path": "C:\\tools;D:\\bin;C:\\tools",
   "pathext": ".com;.exe;.EXE",
   "command": "magick",
   "additional": [
    "E:/x"
   ],
   "candidates": [
    "C:\\tools\\magick.exe",
    "C:\\tools\\magick.EXE",
    "C:\\tools\\magick.com",
    "D:\\bin\\magick.exe",
    "D:\\bin\\magick.EXE",
    "D:\\bin\\magick.com",
    "E:\\x\\magick.exe",
    "E:\\x\\magick.EXE",
    "E:\\x\\magick.com"
   ]
  },
  {
   "windows": true,
   "path": "C:\\tools",
   "pathext": null,
   "command": "gs",
   "additional": [],
   "candidates": [
    "C:\\tools\\gs"
   ]
  },
  {
   "windows": true,
   "path": "C:\\a;;C:\\b",
   "pathext": ".EXE",
   "command": "gs",
   "additional": [],
   "candidates": [
    "C:\\a\\gs.EXE",
    "\\gs.EXE",
    "C:\\b\\gs.EXE"
   ]
  }
 ],
 "fs_layout": [
  {
   "path": "a/tool",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "b/tool",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "noexec/tool",
   "kind": "file",
   "mode": 420
  },
  {
   "path": "dirhit/tool",
   "kind": "dir",
   "mode": 493
  },
  {
   "path": "groupother/tool",
   "kind": "file",
   "mode": 9
  },
  {
   "path": "owneronly/tool",
   "kind": "file",
   "mode": 64
  },
  {
   "path": "link/tool",
   "kind": "symlink",
   "target": "../a/tool"
  },
  {
   "path": "broken/tool",
   "kind": "symlink",
   "target": "../nowhere/tool"
  },
  {
   "path": "linknoexec/tool",
   "kind": "symlink",
   "target": "../noexec/tool"
  },
  {
   "path": "ext/tool.sh",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "ext/tool",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "nested/sub/tool",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "magick/magick",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "gs/gs",
   "kind": "file",
   "mode": 493
  },
  {
   "path": "empty",
   "kind": "dir",
   "mode": 493
  }
 ],
 "fs_find": [
  {
   "name": "first-dir-wins",
   "path": "<ROOT>/a:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/a/tool"
  },
  {
   "name": "order-reversed",
   "path": "<ROOT>/b:<ROOT>/a",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "skip-nonexecutable",
   "path": "<ROOT>/noexec:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "skip-directory",
   "path": "<ROOT>/dirhit:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "group-other-bits-only-not-owner",
   "path": "<ROOT>/groupother:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "owner-exec-bit-only",
   "path": "<ROOT>/owneronly:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/owneronly/tool"
  },
  {
   "name": "symlink-to-executable",
   "path": "<ROOT>/link:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/link/tool"
  },
  {
   "name": "broken-symlink-skipped",
   "path": "<ROOT>/broken:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "symlink-to-nonexecutable-skipped",
   "path": "<ROOT>/linknoexec:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "absent",
   "path": "<ROOT>/empty:<ROOT>/noexec",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": null
  },
  {
   "name": "absent-empty-path",
   "path": "",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": null
  },
  {
   "name": "additional-paths-after-path",
   "path": "<ROOT>/empty",
   "pathext": null,
   "command": "tool",
   "additional": [
    "<ROOT>/b",
    "<ROOT>/a"
   ],
   "found": "<ROOT>/b/tool"
  },
  {
   "name": "path-before-additional",
   "path": "<ROOT>/a",
   "pathext": null,
   "command": "tool",
   "additional": [
    "<ROOT>/b"
   ],
   "found": "<ROOT>/a/tool"
  },
  {
   "name": "trailing-slash-dir",
   "path": "<ROOT>/a/",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/a/tool"
  },
  {
   "name": "double-slash-dir",
   "path": "<ROOT>/a//",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/a//tool"
  },
  {
   "name": "pathext-on-unix-ext-first",
   "path": "<ROOT>/ext",
   "pathext": ".sh",
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/ext/tool.sh"
  },
  {
   "name": "pathext-on-unix-no-bare-name",
   "path": "<ROOT>/a",
   "pathext": ".sh",
   "command": "tool",
   "additional": [],
   "found": null
  },
  {
   "name": "pathext-empty-entry-bare-name",
   "path": "<ROOT>/a",
   "pathext": ";.sh",
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/a/tool"
  },
  {
   "name": "command-with-subdir",
   "path": "<ROOT>/nested",
   "pathext": null,
   "command": "sub/tool",
   "additional": [],
   "found": "<ROOT>/nested/sub/tool"
  },
  {
   "name": "nonexistent-dir-skipped",
   "path": "<ROOT>/missing:<ROOT>/b",
   "pathext": null,
   "command": "tool",
   "additional": [],
   "found": "<ROOT>/b/tool"
  }
 ],
 "strategy": [
  {
   "name": "alias-command-in-path",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/magick",
   "tool": "magick",
   "aliases": [],
   "alias_table": {
    "magick": {
     "target": "convert",
     "definition": "alias magick='convert'"
    }
   },
   "responses": [],
   "alias_probes": [
    "magick"
   ],
   "system_calls": [],
   "events": [
    [
     "alias",
     "magick"
    ],
    [
     "path_scan",
     "magick",
     "<ROOT>/magick/magick"
    ]
   ],
   "result": {
    "path": "<ROOT>/magick/magick",
    "source": "alias",
    "shell": "bash",
    "alias_definition": "alias magick='convert'",
    "description": "Shell alias in bash: alias magick='convert'",
    "alias?": true,
    "path?": false
   }
  },
  {
   "name": "alias-target-in-path",
   "platform": "linux",
   "shell": "zsh",
   "path": "<ROOT>/gs",
   "tool": "ghostscript",
   "aliases": [],
   "alias_table": {
    "ghostscript": {
     "target": "gs",
     "definition": "ghostscript=gs"
    }
   },
   "responses": [],
   "alias_probes": [
    "ghostscript"
   ],
   "system_calls": [],
   "events": [
    [
     "alias",
     "ghostscript"
    ],
    [
     "path_scan",
     "ghostscript",
     null
    ],
    [
     "path_scan",
     "gs",
     "<ROOT>/gs/gs"
    ]
   ],
   "result": {
    "path": "<ROOT>/gs/gs",
    "source": "alias",
    "shell": "zsh",
    "alias_definition": "ghostscript=gs",
    "description": "Shell alias in zsh: ghostscript=gs",
    "alias?": true,
    "path?": false
   }
  },
  {
   "name": "alias-absolute-target-not-resolved",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/empty",
   "tool": "tool",
   "aliases": [],
   "alias_table": {
    "tool": {
     "target": "<ROOT>/a/tool",
     "definition": "alias tool='<ROOT>/a/tool'"
    }
   },
   "responses": [],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "tool"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     null
    ],
    [
     "path_scan",
     "<ROOT>/a/tool",
     null
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     null
    ]
   ],
   "result": null
  },
  {
   "name": "alias-unresolved-falls-to-system",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/empty",
   "tool": "tool",
   "aliases": [],
   "alias_table": {
    "tool": {
     "target": "nothere",
     "definition": "x"
    }
   },
   "responses": [
    [
     [
      "sh",
      [
       "-c",
       "command -v 'tool' 2>/dev/null"
      ]
     ],
     [
      true,
      "/usr/bin/tool\n"
     ]
    ]
   ],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     null
    ],
    [
     "path_scan",
     "nothere",
     null
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ]
   ],
   "result": {
    "path": "/usr/bin/tool",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at /usr/bin/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "system-command-v-first-line-stripped",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/a",
   "tool": "tool",
   "aliases": [],
   "alias_table": {},
   "responses": [
    [
     [
      "sh",
      [
       "-c",
       "command -v 'tool' 2>/dev/null"
      ]
     ],
     [
      true,
      "  /opt/x/tool  \n/usr/bin/tool\n"
     ]
    ]
   ],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ]
   ],
   "result": {
    "path": "/opt/x/tool",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at /opt/x/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "system-which-after-command-v-fails",
   "platform": "macos",
   "shell": "zsh",
   "path": "<ROOT>/a",
   "tool": "tool",
   "aliases": [],
   "alias_table": {},
   "responses": [
    [
     [
      "which",
      [
       "tool"
      ]
     ],
     [
      true,
      "/usr/local/bin/tool\n"
     ]
    ]
   ],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "tool"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "tool"
    ]
   ],
   "result": {
    "path": "/usr/local/bin/tool",
    "source": "path",
    "shell": "zsh",
    "alias_definition": null,
    "description": "Found in PATH at /usr/local/bin/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "system-empty-first-line-then-which",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/a",
   "tool": "tool",
   "aliases": [],
   "alias_table": {},
   "responses": [
    [
     [
      "sh",
      [
       "-c",
       "command -v 'tool' 2>/dev/null"
      ]
     ],
     [
      true,
      "\n/x/tool\n"
     ]
    ],
    [
     [
      "which",
      [
       "tool"
      ]
     ],
     [
      true,
      ""
     ]
    ]
   ],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "tool"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     "<ROOT>/a/tool"
    ]
   ],
   "result": {
    "path": "<ROOT>/a/tool",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at <ROOT>/a/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "system-success-empty-stdout-falls-to-path",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/a",
   "tool": "tool",
   "aliases": [],
   "alias_table": {},
   "responses": [
    [
     [
      "sh",
      [
       "-c",
       "command -v 'tool' 2>/dev/null"
      ]
     ],
     [
      true,
      "   "
     ]
    ]
   ],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "tool"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     "<ROOT>/a/tool"
    ]
   ],
   "result": {
    "path": "<ROOT>/a/tool",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at <ROOT>/a/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "path-scan-last",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/noexec:<ROOT>/b",
   "tool": "tool",
   "aliases": [],
   "alias_table": {},
   "responses": [],
   "alias_probes": [
    "tool"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'tool' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "tool"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "tool"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'tool' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "tool"
    ],
    [
     "path_scan",
     "tool",
     "<ROOT>/b/tool"
    ]
   ],
   "result": {
    "path": "<ROOT>/b/tool",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at <ROOT>/b/tool",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "primary-then-aliases-in-order",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/gs:<ROOT>/magick",
   "tool": "ghostscript",
   "aliases": [
    "gsx",
    "magick",
    "gs"
   ],
   "alias_table": {},
   "responses": [],
   "alias_probes": [
    "ghostscript",
    "gsx",
    "magick"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'ghostscript' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "ghostscript"
     ]
    ],
    [
     "sh",
     [
      "-c",
      "command -v 'gsx' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "gsx"
     ]
    ],
    [
     "sh",
     [
      "-c",
      "command -v 'magick' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "magick"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "ghostscript"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'ghostscript' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "ghostscript"
    ],
    [
     "path_scan",
     "ghostscript",
     null
    ],
    [
     "alias",
     "gsx"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'gsx' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "gsx"
    ],
    [
     "path_scan",
     "gsx",
     null
    ],
    [
     "alias",
     "magick"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'magick' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "magick"
    ],
    [
     "path_scan",
     "magick",
     "<ROOT>/magick/magick"
    ]
   ],
   "result": {
    "path": "<ROOT>/magick/magick",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at <ROOT>/magick/magick",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "primary-wins-over-alias",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/gs:<ROOT>/magick",
   "tool": "magick",
   "aliases": [
    "gs"
   ],
   "alias_table": {},
   "responses": [],
   "alias_probes": [
    "magick"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'magick' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "magick"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "magick"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'magick' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "magick"
    ],
    [
     "path_scan",
     "magick",
     "<ROOT>/magick/magick"
    ]
   ],
   "result": {
    "path": "<ROOT>/magick/magick",
    "source": "path",
    "shell": "bash",
    "alias_definition": null,
    "description": "Found in PATH at <ROOT>/magick/magick",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "nothing-found",
   "platform": "linux",
   "shell": "bash",
   "path": "<ROOT>/empty",
   "tool": "nope",
   "aliases": [
    "nada"
   ],
   "alias_table": {},
   "responses": [],
   "alias_probes": [
    "nope",
    "nada"
   ],
   "system_calls": [
    [
     "sh",
     [
      "-c",
      "command -v 'nope' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "nope"
     ]
    ],
    [
     "sh",
     [
      "-c",
      "command -v 'nada' 2>/dev/null"
     ]
    ],
    [
     "which",
     [
      "nada"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "nope"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'nope' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "nope"
    ],
    [
     "path_scan",
     "nope",
     null
    ],
    [
     "alias",
     "nada"
    ],
    [
     "system",
     "sh",
     "-c",
     "command -v 'nada' 2>/dev/null"
    ],
    [
     "system",
     "which",
     "nada"
    ],
    [
     "path_scan",
     "nada",
     null
    ]
   ],
   "result": null
  },
  {
   "name": "windows-where-exe",
   "platform": "windows",
   "shell": "powershell",
   "path": "<ROOT>/empty",
   "tool": "gswin64c",
   "aliases": [],
   "alias_table": {},
   "responses": [
    [
     [
      "where",
      [
       "gswin64c.exe"
      ]
     ],
     [
      true,
      "C:\\Program Files\\gs\\gs10.02.1\\bin\\gswin64c.exe\r\nC:\\other\\gswin64c.exe\r\n"
     ]
    ]
   ],
   "alias_probes": [
    "gswin64c"
   ],
   "system_calls": [
    [
     "where",
     [
      "gswin64c.exe"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "gswin64c"
    ],
    [
     "system",
     "where",
     "gswin64c.exe"
    ]
   ],
   "result": {
    "path": "C:\\Program Files\\gs\\gs10.02.1\\bin\\gswin64c.exe",
    "source": "path",
    "shell": "powershell",
    "alias_definition": null,
    "description": "Found in PATH at C:\\Program Files\\gs\\gs10.02.1\\bin\\gswin64c.exe",
    "alias?": false,
    "path?": true
   }
  },
  {
   "name": "windows-where-fails-no-which",
   "platform": "windows",
   "shell": "cmd",
   "path": "<ROOT>/empty",
   "tool": "gswin64c",
   "aliases": [
    "gs"
   ],
   "alias_table": {},
   "responses": [],
   "alias_probes": [
    "gswin64c",
    "gs"
   ],
   "system_calls": [
    [
     "where",
     [
      "gswin64c.exe"
     ]
    ],
    [
     "where",
     [
      "gs.exe"
     ]
    ]
   ],
   "events": [
    [
     "alias",
     "gswin64c"
    ],
    [
     "system",
     "where",
     "gswin64c.exe"
    ],
    [
     "path_scan",
     "gswin64c",
     null
    ],
    [
     "alias",
     "gs"
    ],
    [
     "system",
     "where",
     "gs.exe"
    ],
    [
     "path_scan",
     "gs",
     null
    ]
   ],
   "result": null
  }
 ],
 "first_line": [
  {
   "stdout": "",
   "line": null
  },
  {
   "stdout": "\n",
   "line": null
  },
  {
   "stdout": "x",
   "line": "x"
  },
  {
   "stdout": "x\ny",
   "line": "x"
  },
  {
   "stdout": "  x  \ny",
   "line": "x"
  },
  {
   "stdout": "\nx",
   "line": null
  },
  {
   "stdout": "   ",
   "line": null
  },
  {
   "stdout": "\r\nx",
   "line": null
  },
  {
   "stdout": "x\r\n",
   "line": "x"
  },
  {
   "stdout": "a b\n",
   "line": "a b"
  },
  {
   "stdout": "\u0000x\u0000 \u000b",
   "line": "x"
  },
  {
   "stdout": "\u00a0x\n",
   "line": "\u00a0x"
  },
  {
   "stdout": "\tx\f\n",
   "line": "x"
  }
 ],
 "alias_target": [
  {
   "target": "magick",
   "command": "magick"
  },
  {
   "target": "magick -quiet",
   "command": "magick"
  },
  {
   "target": "  ls -la",
   "command": ""
  },
  {
   "target": "\tgs\t-q",
   "command": ""
  },
  {
   "target": "",
   "command": null
  },
  {
   "target": "   ",
   "command": null
  },
  {
   "target": "/opt/bin/x arg",
   "command": "/opt/bin/x"
  },
  {
   "target": "a\u000bb",
   "command": "a"
  },
  {
   "target": "x\u00a0y z",
   "command": "x\u00a0y"
  }
 ],
 "description": [
  {
   "path": "/usr/bin/ffmpeg",
   "source": "path",
   "shell": "bash",
   "alias_definition": null,
   "description": "Found in PATH at /usr/bin/ffmpeg",
   "alias?": false,
   "path?": true
  },
  {
   "path": "/usr/bin/ffmpeg",
   "source": "alias",
   "shell": "zsh",
   "alias_definition": "alias ffmpeg='/opt/homebrew/bin/ffmpeg'",
   "description": "Shell alias in zsh: alias ffmpeg='/opt/homebrew/bin/ffmpeg'",
   "alias?": true,
   "path?": false
  },
  {
   "path": "/usr/bin/ffmpeg",
   "source": "alias",
   "shell": "fish",
   "alias_definition": null,
   "description": "Shell alias in fish: ",
   "alias?": true,
   "path?": false
  },
  {
   "path": "C:\\x\\gs.exe",
   "source": "path",
   "shell": "powershell",
   "alias_definition": "ignored",
   "description": "Found in PATH at C:\\x\\gs.exe",
   "alias?": false,
   "path?": true
  }
 ],
 "executable_name": [
  {
   "executable_name": null,
   "profile_name": "ghostscript",
   "profile_aliases": null,
   "locator_call": {
    "tool_name": "ghostscript",
    "aliases": [],
    "platform": "linux"
   },
   "returned": null
  },
  {
   "executable_name": "gswin64c",
   "profile_name": "ghostscript",
   "profile_aliases": [
    "gs"
   ],
   "locator_call": {
    "tool_name": "gswin64c",
    "aliases": [
     "gs"
    ],
    "platform": "linux"
   },
   "returned": null
  },
  {
   "executable_name": "",
   "profile_name": "ghostscript",
   "profile_aliases": [],
   "locator_call": {
    "tool_name": "",
    "aliases": [],
    "platform": "linux"
   },
   "returned": null
  }
 ]
}"#;
