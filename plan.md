# markdownai - Agent-First Markdown CLI 설계

## Context

AI 에이전트가 대규모 마크다운 문서 컬렉션(Obsidian vault, docs repo 등)을 효율적으로 탐색·조작하기 위한 CLI 도구. jsonai와 동일한 Rust + clap v4 + agent-first 패턴을 따르되, 마크다운 구조적 특성(헤더, 섹션, 내부 링크, 프론트매터)을 반영한다.

**핵심 문제**: `cat`으로 전체 파일을 읽으면 토큰 낭비. 에이전트가 필요한 부분만 정확히 읽고, 검색하고, 수정할 수 있어야 한다.

**출력 기본값**: raw markdown이 기본. `--json` 플래그로 JSON envelope 전환 (에이전트가 구조화된 데이터 필요 시). `--json --pretty` 조합 가능 (기본 compact).
**구현 범위**: 전체 기능을 한번에 구현 (Phase 분리 없이).
**stdin 지원**: 파일 인자에 `"-"` 전달 시 stdin에서 읽기. `toc`, `read`, `links`, `frontmatter` 등 인덱스 불필요 커맨드에서 지원. (예: `git show HEAD:doc.md | markdownai toc -`)
**exit code 규약**: 0=성공(매치 있음), 1=매치 없음/대상 없음, 2=에러. jsonai와 동일.
**프로젝트 루트**: `.git/` 디렉토리가 있는 상위 디렉토리를 자동 탐색. `.worktoolai/`는 프로젝트 루트에 생성. `--root <DIR>`로 명시적 지정 가능.
**INPUT 규칙**: `<INPUT>` 인자는 파일 또는 디렉토리. 디렉토리일 경우 하위 `.md` 파일을 재귀 탐색. `<FILE>` 인자는 단일 파일만. stdin은 `"-"`.

### 글로벌 출력 제어 (jsonai 패턴 적응)

| 플래그 | 타입 | 기본값 | 설명 |
|--------|------|--------|------|
| `--max-bytes <N>` | `Option<usize>` | None | 출력 최대 바이트. 초과 시 결과를 잘라내고 `truncated: true` 설정. JSON 유효성 유지 |
| `--limit <N>` | `usize` | 20 | 결과 항목 수 제한 |
| `--offset <N>` | `usize` | 0 | 결과 시작 위치 (paging) |
| `--threshold <N>` | `usize` | 50 | 오버플로 임계값. 결과가 이를 초과하면 plan 모드로 전환 |
| `--no-overflow` | `bool` | false | 오버플로 보호 우회, 항상 결과 반환 |
| `--plan` | `bool` | false | 강제 plan 모드: 메타데이터/요약만, 결과 없음 |
| `--count-only` | `bool` | false | 결과 본문 없이 건수(meta)만 반환. 모든 목록형 커맨드에서 사용 가능 |
| `--exists` | `bool` | false | 대상 존재 여부만 확인. exit code로 반환 (0=존재, 1=미존재). 출력 없음 |
| `--stats` | `bool` | false | 본문 없이 크기/구조 통계만 반환. 읽기 전 크기 판단용 |
| `--facets <FIELD>` | `Option<String>` | None | 지정 필드의 고유값 분포 반환. frontmatter, search 등에서 사용 |

**truncation 로직** (jsonai `truncate_to_budget` 적응):
1. `--max-bytes` 설정 시, envelope overhead(200B) 제외 후 남은 바이트 내에서 항목을 순차 직렬화
2. 바이트 초과 시 나머지 항목 잘라내고 `meta.truncated = true`
3. `total > limit`일 때도 `meta.truncated = true`

**paging envelope** (모든 목록형 커맨드 공통):
```json
{"meta":{"total":47,"returned":10,"offset":0,"limit":10,"truncated":true,"has_more":true,"next_offset":10}}
```
raw 모드에서는 마지막 줄에:
```
--- 10/47 shown, next: --offset 10 ---
```

**`--exists` 예시**:
```bash
markdownai read doc.md --section "#1.3" --exists  # exit 0 = 있음, exit 1 = 없음
markdownai search ./docs -q "OAuth" --exists       # 매치 존재 여부만
```

**`--stats` 예시** (`markdownai read doc.md --stats`):
```json
{"file":"doc.md","bytes":8192,"lines":210,"sections":12,"code_blocks":5,"has_frontmatter":true,"links":{"wiki":3,"markdown":7}}
```
raw:
```
doc.md: 8192 bytes, 210 lines, 12 sections, 5 code blocks, frontmatter: yes, links: 10
```

**`--stats` 디렉토리** (`markdownai read ./docs --stats`):
```json
{"path":"./docs","files":45,"total_bytes":102400,"total_lines":3200,"total_sections":180}
```

**`--facets` 예시** (`markdownai frontmatter ./docs --facets tags`):
```json
{"field":"tags","total_files":45,"facets":{"rust":12,"auth":5,"api":8,"testing":3}}
```
raw:
```
tags: rust(12) auth(5) api(8) testing(3)  [45 files]
```

**overflow plan envelope** (search, backlinks, frontmatter 등 대량 결과 가능 커맨드):
```json
{"meta":{"total":350,"returned":0,"overflow":true,"threshold":50},"plan":{"suggestion":"add --scope headers or narrow query","facets":{"by_file":{"docs/auth.md":12,"docs/api.md":8}}}}
```

---

## 실행 파이프라인

**모든 커맨드가 동일한 5단계 파이프라인**을 따른다. 직접 파싱 / DB 분기 없이 단일 코드 경로.

```
┌─────────────┐    ┌─────────────┐    ┌──────────┐    ┌──────────────┐    ┌────────────┐
│ 1. 변경 체크 │ → │ 2. DB sync  │ → │ 3. 검색   │ → │ 4. 파일 확인  │ → │ 5. 읽기/출력│
└─────────────┘    └─────────────┘    └──────────┘    └──────────────┘    └────────────┘
```

### Step 1. 변경 체크
- INPUT이 파일이면 해당 파일만, 디렉토리면 하위 `.md` 재귀 탐색
- **hash-only 방식**: 대상 파일의 content hash (xxh3) 를 DB 기록과 비교. mtime에 의존하지 않음
  - 환경 무관 (git clone, rsync, NFS 등 어디서든 동일하게 동작)
  - 1000파일 ~100ms (파일 읽기 + hash 계산)
- **DB 미존재 시**: INPUT 스코프만 인덱싱 (단일 파일 명령에 전체 빌드 방지). 전체 빌드는 `markdownai index` 수동 실행 또는 디렉토리 대상 커맨드 시에만

### Step 2. DB sync
- hash 변경된 파일: DB에서 기존 정보 **삭제** 후 재파싱하여 **등록** (Tantivy + SQLite 양쪽)
- 디스크에서 삭제된 파일: DB에서도 제거
- 변경 없으면 skip (비용 0)
- SQLite + Tantivy를 단일 트랜잭션으로 커밋 (아래 "정합성 보장" 참조)

### Step 3. 대상 해석 (target resolution)
- 읽기/검색 커맨드: DB에서 조회
- 조작 커맨드: 대상 섹션/필드 해석 + 충돌 검증

| 커맨드 | DB 조회 내용 | 비고 |
|--------|-------------|------|
| `toc` | sections 테이블 → 헤더 목록 | **DB만으로 완결** (Step 5 불필요) |
| `read` | sections 테이블 → 라인 범위 | Step 5에서 파일 읽기 |
| `read --summary` | sections 테이블 → 라인 범위 | Step 5에서 각 섹션 첫 N줄 읽기 |
| `search` | Tantivy 전문 검색 → 매치 위치 | snippet은 Step 5에서 파일 읽기 |
| `links` | links 테이블 조회 | **DB만으로 완결** |
| `backlinks` | links 테이블 역방향 조회 | **DB만으로 완결** |
| `graph` | links 테이블 그래프 빌드 | **DB만으로 완결** |
| `frontmatter` | frontmatter 테이블 조회 | **DB만으로 완결** |
| `section-set` | sections 테이블 → 대상 라인 범위 해석 | Step 5에서 파일 쓰기 |
| `section-add` | sections 테이블 → 삽입 위치 해석 | Step 5에서 파일 쓰기 |
| `section-delete` | sections 테이블 → 삭제 범위 해석 | Step 5에서 파일 쓰기 |
| `frontmatter-set` | frontmatter 테이블 → 기존 값 확인 | Step 5에서 파일 쓰기 |

### Step 4. 파일 확인
- 대상 파일 경로 + 라인 범위 확정
- `--count-only`, `--exists`, `--stats` 등 메타 전용 플래그 시 **여기서 종료** (Step 5 불필요)
- DB만으로 완결되는 커맨드 (`toc`, `links`, `backlinks`, `graph`, `frontmatter`) 도 **여기서 종료**

### Step 5. 파일 I/O
- **읽기**: 확정된 라인 범위만 읽어서 출력. `--max-bytes` 적용, paging 메타 부착
- **쓰기**: 조작 커맨드. `--dry-run` 시 변경 내용만 표시하고 쓰기/post-sync 생략
  - 쓰기 직전 파일 hash를 재확인하여 Step 2 이후 외부 변경이 없는지 검증 → 불일치 시 abort (exit 2)
  - 쓰기 완료 후 해당 파일만 즉시 Step 2 재실행 (post-sync)
  - `--output <FILE>` 시 원본은 변경 없음. 출력 파일은 sync 대상 아님
- `--json` 시 envelope, raw 시 마크다운 + `--- N/M shown ---` 꼬리

### fallback (DB 우회)
stdin (`"-"`) 입력 시에만 직접 파싱. DB를 거치지 않는 유일한 예외.

---

## DB 구조

### 저장소
```
.worktoolai/
  markdownai.db        ← SQLite (모든 메타데이터)
  markdownai_index/    ← Tantivy 영속 인덱스 (전문검색 본문)
```
`.worktoolai/`는 여러 도구가 공유하는 디렉토리. markdownai는 자기 네임스페이스만 사용.

### 왜 이중 구조인가?
- **Tantivy**: BM25 스코어링, fuzzy/regex 전문검색. 본문 검색에 특화
- **SQLite (WAL 모드)**: 구조화된 메타데이터 저장소. 향후 embedding 벡터 확장 가능

### SQLite 스키마

**files**
| 컬럼 | 타입 | 설명 |
|------|------|------|
| `id` | INTEGER PK | |
| `path` | TEXT UNIQUE | 프로젝트 루트 기준 상대 경로 |
| `content_hash` | TEXT | xxh3 hash |
| `bytes` | INTEGER | 파일 크기 |
| `lines` | INTEGER | 줄 수 |
| `has_frontmatter` | BOOLEAN | |
| `last_indexed_at` | TEXT | ISO 8601 |
| `sync_epoch` | INTEGER | 정합성 체크용 auto-increment |
| `parse_error` | TEXT NULL | 파싱 실패 시 에러 메시지 |

**sections**
| 컬럼 | 타입 | 설명 |
|------|------|------|
| `id` | INTEGER PK | |
| `file_id` | INTEGER FK → files | ON DELETE CASCADE |
| `parent_id` | INTEGER FK → sections NULL | 상위 섹션 (트리 구조) |
| `section_index` | TEXT | `#1.1` 형식, UNIQUE(file_id, section_index) |
| `ordinal` | INTEGER | 파일 내 출현 순서 (0-based) |
| `level` | INTEGER | 1~6 |
| `title` | TEXT | 헤더 텍스트 (raw) |
| `start_line` | INTEGER | 헤더 라인 |
| `end_line` | INTEGER | 다음 동급/상위 헤더 직전 또는 EOF |

**links**
| 컬럼 | 타입 | 설명 |
|------|------|------|
| `id` | INTEGER PK | |
| `source_file_id` | INTEGER FK → files | ON DELETE CASCADE |
| `source_line` | INTEGER | 링크 위치 |
| `target_raw` | TEXT | 원본 텍스트 (`[[Page]]`, `[text](url)`) |
| `target_path` | TEXT NULL | 정규화된 파일 경로 |
| `target_anchor` | TEXT NULL | `#heading` 앵커 |
| `link_type` | TEXT | `wiki` / `markdown` |
| `resolved_file_id` | INTEGER FK → files NULL | 대상 파일 존재 시 |
| `is_broken` | BOOLEAN | 대상 미존재 시 true |

**frontmatter**
| 컬럼 | 타입 | 설명 |
|------|------|------|
| `id` | INTEGER PK | |
| `file_id` | INTEGER FK → files | ON DELETE CASCADE |
| `key` | TEXT | 필드 이름 |
| `value_json` | TEXT | 원본 JSON 값 |
| `value_type` | TEXT | `string` / `number` / `boolean` / `array` / `object` |
| `value_text` | TEXT NULL | scalar일 때 텍스트 표현 (검색/필터용) |

### sync 제어 플래그

| 플래그 | 설명 |
|--------|------|
| `--sync auto` | **(기본)** hash 비교 → 변경분만 업데이트 후 진행 |
| `--sync force` | DB 삭제 후 전체 재빌드 |

### stale 판단 & 경고

1. Step 1에서 stale 파일 수 계산
2. stale 비율에 따라:
   - **0%**: sync 불필요, 바로 Step 3
   - **<10%**: 증분 sync (stderr에 `synced 5 files`)
   - **10%~50%**: 증분 sync + 경고 (`warning: 15% stale, synced 150 files`)
   - **>50%**: `markdownai index --force` 추천 경고. 증분 sync 실행
3. **DB 미존재**: INPUT 스코프만 자동 빌드 (단일 파일이면 해당 파일만). 전체 빌드는 `markdownai index <PATH>` 수동 실행

### 동시 접근

- SQLite: **WAL 모드** — 읽기/쓰기 동시 가능. 쓰기는 writer lock 직렬화
- Tantivy: single writer lock. 동시 쓰기 시도 시 재시도 3회 → 실패 시 에러 (exit 2)
- lock file: `.worktoolai/markdownai.lock` — 전체 재빌드 시에만 사용

### 정합성 보장 (SQLite ↔ Tantivy)

Step 2 sync 시 두 저장소를 일관되게 유지하기 위한 규칙:

**커밋 순서**: SQLite 먼저 → Tantivy 커밋
- SQLite에 `sync_epoch` (auto-increment) 컬럼을 `files` 테이블에 추가
- Tantivy 각 문서에도 동일한 `sync_epoch` 저장
- 정합성 체크: `files.sync_epoch`와 Tantivy 문서의 `sync_epoch` 비교

**부분 실패 복구**:
| 상황 | 탐지 | 복구 |
|------|------|------|
| SQLite 커밋 성공 + Tantivy 커밋 실패 | Tantivy에 해당 epoch 문서 없음 | 해당 파일만 Tantivy 재인덱싱 |
| SQLite 손상 (`SQLITE_CORRUPT`) | DB open 실패 | stderr 경고 + `--sync force`로 자동 전체 재빌드 |
| Tantivy 인덱스 손상 | reader open 실패 | stderr 경고 + Tantivy 디렉토리 삭제 후 SQLite 기반으로 재빌드 |
| 파일 파싱 실패 (malformed frontmatter 등) | 파서 에러 | `files.parse_error`에 기록. 해당 파일은 부분 인덱싱 (파싱 가능한 부분만). stderr 경고 |

**자동 복구 원칙**: 읽기 커맨드에서 DB 손상 감지 시 자동 재빌드 시도. 쓰기 커맨드에서는 abort (exit 2) + 사용자에게 `markdownai index --force` 안내.

### `markdownai index <PATH>`

수동 DB 관리.
```
  --force          전체 재빌드 (DB 삭제 후 처음부터)
  --status         sync 없이 현황만 표시
  --dry-run        변경될 파일 목록만 표시, 실제 업데이트 없음
  --check          SQLite ↔ Tantivy 정합성 검증만 수행
```

`--status` (raw):
```
db: .worktoolai/markdownai.db (last sync: 2025-01-15 14:30:02)
files: 1,234 indexed, 12 stale, 3 deleted
size: sqlite 1.1MB, tantivy 4.2MB
```
`--status` (--json):
```json
{"path":".worktoolai/markdownai.db","last_sync":"2025-01-15T14:30:02Z","files":{"indexed":1234,"stale":12,"deleted":3,"untracked":5},"size":{"sqlite_bytes":1153434,"tantivy_bytes":4404019}}
```

---

## Help 설계

에이전트가 `--help` 한번으로 이해할 수 있되 토큰을 최소화한다.
clap의 `about` + `after_help`(예시)를 활용. `long_about`은 사용하지 않음.

### `markdownai --help`

```
markdownai - Agent-first Markdown CLI (auto-syncs DB, raw output default)

Section: "#1.1" (toc index) | "## Head > ### Sub" (path) | "L10-L25" (lines)
  toc FILE                 headings with section numbers
  read FILE                content (--section ADDR --summary [N] --meta)
  tree PATH                directory structure
  search INPUT -q QUERY    full-text (multi -q, --scope, --match, --context)
  frontmatter INPUT        YAML fields (--field --filter --facets FIELD)
  links FILE               outgoing links (--broken --resolved)
  backlinks FILE           incoming links
  graph INPUT              link graph (--format adjacency|edges|stats)
  section-set FILE -s ADDR replace section (-c TEXT | --content-file F | --content -)
  section-add FILE -t HDR  add section (--after --before --level)
  section-delete FILE -s ADDR
  frontmatter-set FILE -k KEY -v VAL
  index PATH               DB management (--status --force --check)
Flags: --json --max-bytes N --limit N --offset N --count-only --exists --stats
Exit: 0=ok 1=not-found 2=error | Input: file, dir (recursive .md), "-" (stdin)
```

### 서브커맨드 `--help`

각 서브커맨드는 `about` 1줄 + `after_help`에 예시 2~3개만:
```
markdownai-read: Read file or section

  read doc.md                        # full file
  read doc.md --section "#1.1"       # toc number / "## Setup" / "L10-L25"
  read doc.md --summary              # first 3 lines per section
  read doc.md --stats                # size/structure only
```

---

## 커맨드 설계

### 1. 구조 탐색

#### `markdownai toc <FILE>`
헤더 계층 구조를 **번호 인덱스와 함께** 출력. `--depth`, `--flat`, `--limit`, `--offset` 옵션.
기본 출력 (raw):
```
1   # Project                        (L1)
1.1 ## Setup                         (L5)
1.1.1 ### Prerequisites              (L8)
1.2 ## Usage                         (L20)
```
번호는 `read --section "#1.1"` 등에서 바로 사용 가능. `(L숫자)`는 라인 번호.
paging 적용 시 (raw):
```
1   # Project                        (L1)
1.1 ## Setup                         (L5)

--- 2/15 headers shown, next: --offset 2 ---
```
```json
{"meta":{"file":"README.md","total":15,"returned":2,"offset":0,"has_more":true,"next_offset":2},"results":[{"index":"1","level":1,"text":"Project","line":1},{"index":"1.1","level":2,"text":"Setup","line":5}]}
```

#### `markdownai read <FILE>`
파일 전체 또는 특정 섹션만 읽기. 핵심 기능.
- `--section "## Setup > ### Prerequisites"` : 섹션 주소로 특정 부분만 읽기
- `--section "#1.1"` : toc 번호 인덱스로 섹션 지정 (toc 출력과 1:1 대응)
- `--summary [N]` : 각 섹션의 첫 N줄만 프리뷰 (기본 3줄). 전체를 읽지 않고 내용 파악 용도. `--limit`, `--offset`으로 섹션 단위 paging
- `--json` : JSON envelope로 출력 (기본은 raw markdown)
- `--max-bytes` : 바이트 버짓. 초과 시 잘라내고 마지막에 truncation 표시
- `--meta` : frontmatter 포함

`--max-bytes` 초과 시 (raw):
```
## Setup

Rust 1.75+ 필요. 아래 Prerequisites 참고.
설치 방법은 다음과 같다...

--- truncated at L42, 2048/8192 bytes, next: --section "L43-" ---
```

`--max-bytes` 초과 시 (--json):
```json
{"meta":{"file":"README.md","truncated":true,"bytes_shown":2048,"bytes_total":8192,"next_line":43},"content":"## Setup\n\nRust 1.75+ 필요..."}
```

`--summary` 예시 (raw):
```
## Setup (L5, #1.1)
  Rust 1.75+ 필요. 아래 Prerequisites 참고.
  ...

### Prerequisites (L8, #1.1.1)
  - rustup 설치
  ...

--- 3/12 sections shown, next: --offset 3 ---
```

`--summary` 예시 (--json):
```json
{"meta":{"file":"README.md","total_sections":12,"returned":3,"offset":0,"has_more":true,"next_offset":3},"results":[{"index":"#1.1","title":"## Setup","line":5,"preview":"Rust 1.75+ 필요. 아래 Prerequisites 참고."}]}
```

#### `markdownai tree <PATH>`
디렉토리 구조를 JSON 트리로. `--depth`, `--files-only`, `--count`

#### `markdownai frontmatter <INPUT>`
YAML frontmatter 파싱/필터링. `--field`, `--filter "tags contains \"rust\""`, `--list`, `--limit`, `--offset`

### 2. 검색

#### `markdownai search <INPUT> -q <QUERY> [-q <QUERY2> ...]`
전문 검색. jsonai의 search와 동일한 패턴. stdin 지원 (`"-"`).
- `-q`를 여러 번 지정하여 **다중 검색** 가능. 각 쿼리별 결과를 그룹으로 반환
- `--match {text,exact,fuzzy,regex}`, `--scope {all,body,headers,frontmatter,code}`
- `--limit`, `--offset`, `--count-only`, `--bare`, `--max-bytes`
- 오버플로 보호: `--threshold`, `--plan`, `--no-overflow`
- `--context` : 매치 주변 N줄

단일 쿼리 결과 envelope (--json):
```json
{"meta":{"query":"OAuth","total":47,"returned":10,"offset":0,"limit":10,"truncated":true,"has_more":true,"next_offset":10},"results":[{"file":"docs/auth.md","section_index":"#1.2","section_title":"## OAuth Flow","line":15,"snippet":"OAuth 2.0을 사용한 인증...","score":0.85}]}
```
`section_index`는 toc 번호와 1:1 대응 → `read --section "#1.2"`로 바로 연결.

다중 쿼리 결과 (`-q OAuth -q JWT`, --json):
```json
{"meta":{"queries":2},"groups":[{"query":"OAuth","meta":{"total":47,"returned":10,"truncated":true,"has_more":true,"next_offset":10},"results":[{"file":"docs/auth.md","section_index":"#1.2","line":15,"snippet":"OAuth 2.0을 사용한...","score":0.85}]},{"query":"JWT","meta":{"total":5,"returned":5,"truncated":false,"has_more":false},"results":[{"file":"docs/token.md","section_index":"#1.1","line":3,"snippet":"JWT 토큰 검증...","score":0.92}]}]}
```

`--count-only` 결과 (--json):
```json
{"meta":{"query":"OAuth","total":47}}
```
다중 쿼리 + `--count-only`:
```json
{"meta":{"queries":2},"counts":[{"query":"OAuth","total":47},{"query":"JWT","total":5}]}
```

raw 모드 출력:
```
docs/auth.md:#1.2 ## OAuth Flow (L15, score:0.85)
  OAuth 2.0을 사용한 인증...

--- 10/47 shown, next: --offset 10 ---
```
raw + `--count-only`:
```
OAuth: 47
JWT: 5
```

### 3. 링크 & 그래프

#### `markdownai links <FILE>`
파일의 아웃고잉 링크. `[[wikilink]]`와 `[md](link)` 모두 파싱.
- `--type {wiki,markdown,all}`, `--resolved`, `--broken`, `--limit`, `--offset`

#### `markdownai backlinks <FILE>`
해당 파일을 참조하는 문서 목록. `--limit`, `--offset`

#### `markdownai graph <INPUT>`
링크 그래프 출력. `--start <FILE>` + `--depth N`으로 서브그래프 추출 가능.
- `--format {adjacency,edges,stats}`, `--orphans`, `--limit`, `--offset`

**adjacency** (기본) — 간선 1줄 1개, 파싱 명확
raw:
```
index.md > docs/auth.md
index.md > docs/api.md
docs/auth.md > concepts/OAuth.md
--- 3/7 edges shown, next: --offset 3 ---
```
--json:
```json
{"meta":{"nodes":5,"edges":7},"graph":{"index.md":{"out":["docs/auth.md","docs/api.md"],"in":[]},"docs/auth.md":{"out":["concepts/OAuth.md"],"in":["index.md"]}}}
```

**edges** — 타입/라인 정보 포함
raw:
```
index.md > docs/auth.md wiki L5
index.md > docs/api.md markdown L12
docs/auth.md > concepts/OAuth.md markdown L8
```
--json:
```json
{"meta":{"edges":7},"edges":[{"from":"index.md","to":"docs/auth.md","type":"wiki","line":5}]}
```

**stats** — 통계만
raw:
```
nodes: 5, edges: 7, orphans: 1
most linked: docs/auth.md (3 in)
most linking: index.md (4 out)
```

### 4. 조작

모든 조작 커맨드 공통 옵션: `--dry-run`, `--output <FILE>`, `--with-toc` (변경 후 toc를 응답에 포함, JSON 모드 시 항상 포함).
조작 완료 후 파이프라인 Step 2를 즉시 실행하여 해당 파일의 DB를 자동 sync.

#### `markdownai section-set <FILE> -s <SECTION> -c <CONTENT>`
특정 섹션 내용 교체.
- `-c <CONTENT>` : 짧은 인라인 내용
- `--content-file <FILE>` : 파일에서 내용 읽기
- `--content -` : stdin에서 내용 읽기 (multiline 마크다운용)

#### `markdownai section-add <FILE> -t <TITLE> -c <CONTENT>`
새 섹션 추가. `--after`, `--before`, `--level`
- content 입력: `section-set`과 동일 (`-c`, `--content-file`, `--content -`)

#### `markdownai section-delete <FILE> -s <SECTION>`
섹션 삭제 (헤더 + 하위 내용 전체).

#### `markdownai frontmatter-set <FILE> -k <KEY> -v <VALUE>`
frontmatter 필드 설정/수정. frontmatter 미존재 시 자동 생성.

### 5. DB 관리

#### `markdownai index <PATH>`
DB 빌드/업데이트. 상세는 "DB 구조 > `markdownai index`" 섹션 참조.

---

## 섹션 주소 체계

에이전트가 문서의 특정 부분을 지정하는 3가지 방법:

| 방식        | 형식                      | 예시                             | 비고                          |
| ----------- | ------------------------- | -------------------------------- | ----------------------------- |
| 헤더 경로   | `"## Parent > ### Child"` | `"## Setup > ### Prerequisites"` | 사람이 읽기 쉬움              |
| 위치 인덱스 | `"#N.M"`                  | `"#1.1"` (1번째 h1의 1번째 h2)   | **toc 출력 번호와 1:1 대응**  |
| 라인 범위   | `"L10-L25"`               | 10~25라인                        | 정밀 범위 지정                |

**번호 인덱스 규칙**: 헤더 이름과 무관하게 **출현 순서대로 seq increment**. 동일 이름 헤더가 있어도 번호로 유일하게 식별된다.
```markdown
## FAQ          → #1.1
### Question    → #1.1.1
### Question    → #1.1.2   ← 동일 이름이지만 번호가 다름
## API          → #1.2
```

**추천 워크플로우**: `toc` → 번호 확인 → `read --section "#1.1"` 또는 `read --summary`로 프리뷰 → 필요한 섹션만 정밀 읽기.
**주의**: 번호 인덱스는 문서 편집 시 변경될 수 있다. 조작 커맨드에 `--with-toc`를 사용하면 변경 후 최신 toc를 함께 받을 수 있다.

섹션 범위: 헤더 라인부터 다음 동급/상위 헤더 직전(또는 EOF)까지.

---

## 모듈 구조

```
src/
  main.rs          ← 엔트리포인트, 커맨드 디스패치
  cli.rs           ← clap v4 정의 (모든 커맨드, 인자, enum)
  markdown.rs      ← pulldown-cmark 기반 헤더/섹션 파싱
  section.rs       ← 섹션 주소 파싱, 읽기/교체/삽입/삭제
  engine.rs        ← Tantivy 검색 엔진 (jsonai engine.rs 적응)
  index.rs         ← DB 관리 (.worktoolai/markdownai.db + markdownai_index/)
  links.rs         ← 링크 파싱, 그래프 빌드
  manipulate.rs    ← 파일 조작 (section-*, frontmatter-set)
  output.rs        ← JSON envelope, plan mode, byte budget (jsonai 적응)
  frontmatter.rs   ← YAML frontmatter 파싱/필터/직렬화
```

## 주요 의존성

```toml
[dependencies]
clap = { version = "4", features = ["derive"] }
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
tantivy = "0.22"
pulldown-cmark = "0.12"
rusqlite = { version = "0.32", features = ["bundled"] }
xxhash-rust = { version = "0.8", features = ["xxh3"] }
regex = "1"
glob = "0.3"
walkdir = "2"
```

## 구현 순서 (전체 한번에, 빌드 가능한 순서)

1. **프로젝트 스캐폴딩**: Cargo.toml, main.rs, cli.rs (모든 커맨드 정의)
2. **output.rs**: JSON envelope + raw 출력 모드 (--json 글로벌 플래그)
3. **markdown.rs + section.rs**: 헤더/섹션 파싱, 섹션 주소 체계
4. **frontmatter.rs**: YAML frontmatter 파싱/필터/직렬화
5. **links.rs**: 링크 파싱, 그래프 빌드
6. **index.rs + engine.rs**: SQLite + Tantivy 영속 인덱스, 검색 엔진
7. **manipulate.rs**: 섹션/frontmatter 조작
8. **main.rs 완성**: 모든 커맨드 디스패치 연결
9. **testdata + 테스트**: 샘플 마크다운 + 통합 테스트
10. **CI/CD**: GitHub Actions (jsonai 패턴 적응)

## 참조할 jsonai 파일
- `/Users/bjm/work/ai/jsonai/src/cli.rs` → clap 패턴
- `/Users/bjm/work/ai/jsonai/src/output.rs` → Envelope/Meta 패턴
- `/Users/bjm/work/ai/jsonai/src/engine.rs` → Tantivy 패턴 (in-memory → persistent 적응)
- `/Users/bjm/work/ai/jsonai/src/manipulate.rs` → 조작 패턴
- `/Users/bjm/work/ai/jsonai/.github/workflows/release.yml` → CI/CD 패턴

## 검증 방법
1. `cargo build --release` 성공
2. testdata/ 마크다운 파일로 각 커맨드 수동 테스트
3. 인덱스 생성 → 파일 수정 → 증분 업데이트 확인
4. 대량 파일(1000+)에서 인덱스 성능 확인
5. `cargo test` 통과
