import type { Snapshot } from './types';
export const demo: Snapshot = {
  "repoId": "demo",
  "name": "ui-reference-repo",
  "branch": "main",
  "revision": "demo",
  "hasMore": false,
  "writeBlock": null,
  "recoveryPending": false,
  "commits": [
    {
      "oid": "b62da9c14c4e05000165d06141b6cf28c6a12cb1",
      "parents": [
        "0113069d8c18c03217955901c759c0392419f1a2"
      ],
      "summary": "Follow system theme preference",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "main"
      ]
    },
    {
      "oid": "f0030fdbbbce9ff717aaa8ad4d2d6a6c1d278adf",
      "parents": [
        "0113069d8c18c03217955901c759c0392419f1a2"
      ],
      "summary": "Define sidebar groups",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "feature/sidebar"
      ]
    },
    {
      "oid": "b7003cdcad5fe4aa325da999ad2570c363f70a2f",
      "parents": [
        "0113069d8c18c03217955901c759c0392419f1a2"
      ],
      "summary": "Switch default theme to light",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "feature/theme"
      ]
    },
    {
      "oid": "07ecbda17ac7cb59b2a3694ba477dbb52ae8bc67",
      "parents": [
        "0113069d8c18c03217955901c759c0392419f1a2"
      ],
      "summary": "WIP navigation experiment",
      "author": "UI Research",
      "time": 1795032872,
      "refs": []
    },
    {
      "oid": "0113069d8c18c03217955901c759c0392419f1a2",
      "parents": [
        "20f0d647b56cdb1555d84874d8233bdd08501d18",
        "a796d7b0840c6937c714f0da3afad737da4dd269"
      ],
      "summary": "Merge search improvements",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "v0.2.0"
      ]
    },
    {
      "oid": "20f0d647b56cdb1555d84874d8233bdd08501d18",
      "parents": [
        "0a66ac4b914bf923ca85011c42d29f6ee706bfaf"
      ],
      "summary": "Add release notes",
      "author": "UI Research",
      "time": 1795032872,
      "refs": []
    },
    {
      "oid": "a796d7b0840c6937c714f0da3afad737da4dd269",
      "parents": [
        "77c0405bdb58ef3d7792d065199371b64a226e41"
      ],
      "summary": "Document search acceptance cases",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "feature/search"
      ]
    },
    {
      "oid": "77c0405bdb58ef3d7792d065199371b64a226e41",
      "parents": [
        "0a66ac4b914bf923ca85011c42d29f6ee706bfaf"
      ],
      "summary": "Add case-insensitive search",
      "author": "UI Research",
      "time": 1795032872,
      "refs": []
    },
    {
      "oid": "0a66ac4b914bf923ca85011c42d29f6ee706bfaf",
      "parents": [],
      "summary": "Initialize Atlas Notes sample data",
      "author": "UI Research",
      "time": 1795032872,
      "refs": [
        "v0.1.0"
      ]
    }
  ],
  "stashes": [],
  "tags": [
    {"name": "v0.2.0", "oid": "0113069d8c18c03217955901c759c0392419f1a2", "annotated": false},
    {"name": "v0.1.0", "oid": "0a66ac4b914bf923ca85011c42d29f6ee706bfaf", "annotated": false}
  ],
  "branches": [
    {
      "name": "main",
      "oid": "b62da9c14c4e05000165d06141b6cf28c6a12cb1",
      "current": true
    },
    {
      "name": "feature/sidebar",
      "oid": "f0030fdbbbce9ff717aaa8ad4d2d6a6c1d278adf",
      "current": false
    },
    {
      "name": "feature/theme",
      "oid": "b7003cdcad5fe4aa325da999ad2570c363f70a2f",
      "current": false
    },
    {
      "name": "feature/search",
      "oid": "a796d7b0840c6937c714f0da3afad737da4dd269",
      "current": false
    }
  ],
  "changes": [
    {
      "id": "demo-1",
      "path": "docs/review-checklist.txt",
      "staged": false,
      "unstaged": true,
      "status": "added"
    },
    {
      "id": "demo-2",
      "path": "src/search.ts",
      "staged": false,
      "unstaged": true,
      "status": "modified"
    },
    {
      "id": "demo-3",
      "path": "README.md",
      "staged": true,
      "unstaged": false,
      "status": "modified"
    }
  ]
};
export const demoDiff = "diff --git a/src/search.ts b/src/search.ts\nindex 06d9a22..fb0c69b 100644\n--- a/src/search.ts\n+++ b/src/search.ts\n@@ -1,4 +1,4 @@\n export function search(items: string[], query: string) {\n-  const normalized = query.toLowerCase();\n+  const normalized = query.trim().toLowerCase();\n   return items.filter(item => item.toLowerCase().includes(normalized));\n }\n";

// Synthetic preview patches, never read from or applied to a local repository.
export function demoFileDiff(id:string):string {
 if(id==='demo-2')return demoDiff;
 if(id==='demo-1')return ['diff --git a/docs/review-checklist.txt b/docs/review-checklist.txt','new file mode 100644','--- /dev/null','+++ b/docs/review-checklist.txt','@@ -0,0 +1,3 @@','+Prototype review checklist','+Review changed files','+Verify search behavior',''].join('\n');
 if(id==='demo-3')return ['diff --git a/README.md b/README.md','--- a/README.md','+++ b/README.md','@@ -1,2 +1,3 @@',' # Atlas Notes',' Sample project for UI research.','+Search supports case-insensitive queries.',''].join('\n');
 return 'No sample diff is available for this file.';
}
