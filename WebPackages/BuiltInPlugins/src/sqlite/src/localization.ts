//
//  Copyright (c) 2026 Viva Republica, Inc.
//

import { createTranslator } from "@necto/bridge";

export const t = createTranslator({
  ko: {
    Database: "데이터베이스",
    Refresh: "새로 고침",
    Tables: "테이블",
    view: "뷰",
    "No SQLite files in the app sandbox": "앱 샌드박스에 SQLite 파일이 없어요",
    "{size} · modified {time}": "{size} · {time} 수정",
    "SELECT * FROM …  (⌘↩ to run)": "SELECT * FROM …  (⌘↩ 실행)",
    Run: "실행",
    "Row limit": "행 제한",
    "{count} rows": "{count}행",
    "Running…": "실행 중…",
    "{count} rows (limit reached) · {ms} ms": "{count}행 (제한 도달) · {ms} ms",
    "{count} rows · {ms} ms": "{count}행 · {ms} ms",
    "Pick a table or run a query": "테이블을 고르거나 쿼리를 실행하세요",
    "No database selected": "선택한 데이터베이스가 없어요",
    "Read-only — statements that write are refused.": "읽기 전용이라 쓰기 문장은 거부돼요.",
    "This plugin only works inside Necto": "이 플러그인은 Necto 안에서만 동작해요.",
  },
});
