# NAS Sync for Obsidian

[English](./README.md) | **한국어**

내 NAS에서 직접 돌리는 Obsidian 실시간 동기화. Rust로 작성된 단일 바이너리
서버가 NAS에서 돌고, Obsidian 플러그인이 모든 기기를 동기화합니다.

## 기능

- ⚡ **준실시간 동기화** — 파일 변경이 약 5초 안에 모든 기기에 반영
  (디바운스 + WebSocket 푸시)
- 🔁 **멀티 디바이스** — macOS, Windows, Linux, Android, iOS
  (Obsidian 커뮤니티 플러그인이 도는 곳 어디든)
- 🛟 **데이터 손실 0 충돌 처리** — 동시 수정이 발생해도 어떤 버전도
  사라지지 않습니다. 마지막 저장이 활성본이 되고, 진 버전은 서버에
  보존되어 어느 기기에서든(또는 웹 콘솔에서) 해결 가능
- 🗑 **휴지통 보존** — 삭제된 파일은 NAS에 30일(설정 가능) 보관, 원클릭 복구
- 📜 **로그** — 모든 생성/수정/삭제/충돌이 기록됨
- 🖥 **웹 관리 콘솔** — 활동 로그, 파일 검색+미리보기, 충돌 해결,
  휴지통 복구, 디바이스 관리 (한국어/영어)
- 🪶 **가벼움** — 정적 바이너리 하나, 컨테이너 ~15MB, SQLite,
  백그라운드 폴링 없음

## 동작 방식

```
 Obsidian (맥) ────┐                       ┌──── Obsidian (폰)
                   │  HTTP (파일, ETag)    │
                   ├────────► NAS ◄───────┤
                   │     WebSocket        │
 Obsidian (PC) ────┘    (변경 이벤트)        └──── 웹 관리 콘솔
```

- NAS가 보관함의 원본을 가집니다. `trash/`, `conflicts/`, SQLite 인덱스
  (`meta.db`)도 함께.
- 모든 파일은 BLAKE3 ETag를 가지며, 쓰기는 compare-and-swap(`If-Match`)
  방식 — 오래된 상태에서의 쓰기는 조용히 덮어쓰지 않고 충돌 플로우로
  들어갑니다.
- 클라이언트는 영속 큐를 가지고 있어 오프라인 중 수정도 재연결 시
  업로드됩니다. Obsidian이 꺼진 동안 바뀐 파일은 시작 시 mtime/크기
  기준선 비교로 감지합니다.

### 충돌 정책

두 기기가 같은 파일을 같은 시간대에 수정하면:

1. **마지막 저장이 이깁니다** — 작업 흐름이 멈추지 않습니다.
2. 진 버전은 **서버에 보존**됩니다 (중복 파일로 기기들에 퍼지지 않음).
3. 모든 기기에 충돌 알림이 갑니다 (상태바 + 알림). 플러그인의 비교 모달
   또는 웹 콘솔에서 해결: *현재 유지* / *다른 버전 사용* / *둘 다 보관*.
4. 수정 vs 삭제 충돌: **수정이 이깁니다.** 삭제가 수정 내용을 지우는 일은
   없습니다.

## 빠른 시작

### 1. 서버 (Docker)

```bash
git clone https://github.com/Beomjin4/nas-sync.git nas-sync && cd nas-sync

cp .env.example .env
# ONS_JWT_SECRET, ONS_PAIRING_CODE, ONS_ADMIN_PASSWORD 설정 — .env.example 참고

mkdir -p data
docker compose up -d --build
curl http://localhost:8080/health   # → {"status":"ok", ...}
```

릴리스마다 GHCR에 미리 빌드된 이미지가 올라갑니다 — 소스 빌드 대신
`ghcr.io/Beomjin4/nas-sync`를 쓰려면 `docker-compose.yml`의 주석을 참고하세요.

시놀로지/QNAP 안내, 리버스 프록시·TLS 설정: [DEPLOY.ko.md](./DEPLOY.ko.md)

### 2. 플러그인

커뮤니티 스토어 등록 전까지는 수동 설치:

1. [최신 릴리스](../../releases/latest)에서 `nas-sync.zip` 다운로드
2. `<보관함>/.obsidian/plugins/` 안에 압축 해제
   (`plugins/nas-sync/`에 `main.js` + `manifest.json`이 들어가도록)
3. Obsidian → 설정 → 커뮤니티 플러그인 → **NAS Sync** 활성화
4. 플러그인 설정에서 서버 URL과 `.env`의 페어링 코드 입력 →
   **Pair this device**

**온보딩 동작 방식:** **최초로 페어링한 기기**의 보관함 전체가 NAS에
업로드되며, 그것이 원본이 됩니다. 이후 페어링하는 기기들은 첫 연결 시
그 보관함을 즉시 내려받습니다.

> ⚠ **두 번째 기기부터는 빈 보관함에 페어링하세요.** 새로 페어링하는
> 기기의 보관함에 같은 경로의 파일이 이미 있으면, 첫 동기화 때 **서버
> 버전으로 덮어써집니다.** 서버에 없는 경로의 파일은 업로드되어
> 보존되지만, 경로가 겹치는 로컬 파일은 새 기기 쪽이 이기지 못합니다.
> 확실하지 않으면 먼저 백업하세요.

### 3. 관리 콘솔

`http://<NAS 주소>:8080/admin` 접속 (또는 플러그인 설정의
**Open admin console** 버튼). `ONS_ADMIN_PASSWORD`로 로그인.

## 설정

전부 환경변수로 ([.env.example](./.env.example) 참고):

| 변수 | 기본값 | 용도 |
|---|---|---|
| `ONS_JWT_SECRET` | — (필수) | 디바이스 토큰 서명 |
| `ONS_PAIRING_CODE` | 미설정 (페어링 비활성) | 기기 등록용 사전 공유 코드 |
| `ONS_ADMIN_PASSWORD` | 미설정 (콘솔 비활성) | 웹 콘솔 로그인 |
| `ONS_TRASH_TTL_DAYS` | `30` | 휴지통 보관 일수 |
| `ONS_MAX_FILE_SIZE_MB` | `100` | 업로드 크기 제한 |
| `ONS_BIND` | `0.0.0.0:8080` | 수신 주소 |

## 보안 모델

**신뢰할 수 있는 LAN 또는 VPN** 환경을 전제로 설계됐습니다
(Tailscale/WireGuard와 잘 맞습니다):

- 디바이스 인증: 사전 공유 페어링 코드 → 기기별 JWT, 콘솔에서 개별 차단 가능
- 관리 콘솔: 비밀번호 + HttpOnly 세션 쿠키
- 서버는 평문 HTTP — LAN 밖에 노출하려면 **앞단에 TLS**(리버스 프록시)를
  두세요. 인터넷 직접 노출은 권장하지 않습니다.

## ⚠ 사용 범위

**1인 멀티 디바이스** 용도로 만들어졌습니다. 여러 사람이 동시에 편집해도
동작은 하지만, 충돌이 발생할 수 있습니다.
실시간 공동 편집이 필요하면
[Self-hosted LiveSync](https://github.com/vrtmrz/obsidian-livesync)나
[Relay](https://relay.md)를 권합니다.

## 개발

```
server/   Rust (axum + tokio + sqlx/SQLite + blake3)
plugin/   TypeScript (esbuild) — 에디터 주입 없이 vault 이벤트만 사용
```

```bash
# 서버
cd server && cargo test && cargo run

# 플러그인
cd plugin && npm install && npm run dev
```

릴리스는 자동화되어 있습니다: `v*` 태그를 푸시하면 서버 바이너리, GHCR
이미지, 플러그인 번들이 빌드됩니다 (`.github/workflows/release.yml` 참고).

## 라이선스

[MIT](./LICENSE)
