# 설치 및 첫 사용 가이드

[English](./DEPLOY.md) | **한국어**

시놀로지 NAS에서 서버를 띄우고 Obsidian 보관함에 플러그인을 설치하는
과정을 안내합니다.

---

## 시놀로지에서 서버 띄우기 (Container Manager)

### 방법 A — SSH + docker compose (추천, 반복 작업이 빠름)

DSM에서 SSH 활성화 (`제어판 → 터미널 및 SNMP → SSH 서비스 활성화`) 후:

```bash
ssh 사용자명@<NAS IP>

sudo mkdir -p /volume1/docker/obsidian-nas
sudo chown "$USER" /volume1/docker/obsidian-nas
cd /volume1/docker

git clone https://github.com/Beomjin4/nas-sync.git obsidian-nas
cd obsidian-nas

cp .env.example .env
# 실제 비밀값 생성:
sed -i "s|^ONS_JWT_SECRET=.*|ONS_JWT_SECRET=$(openssl rand -hex 48)|" .env
sed -i "s|^ONS_PAIRING_CODE=.*|ONS_PAIRING_CODE=$(openssl rand -hex 8)|" .env

# 페어링 코드 확인 — 클라이언트에서 필요합니다.
grep ONS_PAIRING_CODE .env

# 시놀로지 Docker는 바인드 마운트 폴더를 자동 생성하지 않습니다.
# 첫 시작 전에 data/를 만들어 두세요:
mkdir -p data

sudo docker compose up -d --build
sudo docker compose logs -f obsidian-nas
```

첫 빌드는 몇 분 걸립니다 (빌더 스테이지에서 Rust musl 컴파일).
뜨고 나면:

```bash
curl http://localhost:8080/health
# {"service":"obsidian-nas-server","status":"ok"}
```

### 방법 B — Container Manager UI

DSM 7.2+ Container Manager는 컴포즈 프로젝트를 지원합니다:

1. Container Manager → 프로젝트 → 생성
2. **이름**: `obsidian-nas`
3. **경로**: `/volume1/docker/obsidian-nas`
   (`docker-compose.yml`과 `.env`가 함께 있어야 함)
4. **소스**: "기존 docker-compose.yml 사용"
5. 빌드 → 시작

로그는 SSH에서 `sudo docker compose logs -f obsidian-nas`로 볼 수 있습니다.

### 데이터 위치

모든 데이터는 `/volume1/docker/obsidian-nas/data/` 아래에 있습니다:

```
data/
├── vault/        실제 파일 (Obsidian 보관함의 미러)
├── trash/        삭제된 파일, 30일 보관
├── conflicts/    충돌에서 진 버전들
└── meta.db       SQLite: 파일 / 디바이스 / 로그 / 충돌 / 휴지통
```

`data/` 디렉토리만 백업하면 서버 전체가 백업됩니다.

### 자주 걸리는 것들

- **8080 포트가 이미 사용 중**: `docker-compose.yml`의 `"8080:8080"`을
  `"8089:8080"` 등으로 바꾸고 플러그인에서 `:8089`를 쓰세요.
- **data/ 권한 오류**: 컨테이너는 기본적으로 root로 동작합니다
  (시놀로지 ACL이 일반 uid를 자주 막기 때문). docker-compose.yml에
  `user:`를 지정해 하드닝했다면 `data/` 소유자를 해당 uid로 맞춰주세요.
- **빌드 중 네트워크 오류**: 첫 빌드 때 cargo가 crates.io에서 의존성을
  받아야 합니다. NAS가 인터넷에 연결돼 있는지 확인하세요.

---

## 플러그인 (Obsidian)

### 1. 보관함에 플러그인 넣기

릴리스 zip을 쓰는 경우: `nas-sync.zip`을 받아
`<보관함>/.obsidian/plugins/`에 압축 해제.

저장소에서 직접 복사하는 경우:

```bash
VAULT="/path/to/your/vault"
mkdir -p "$VAULT/.obsidian/plugins/nas-sync"
cp plugin/manifest.json plugin/main.js "$VAULT/.obsidian/plugins/nas-sync/"
```

(`/path/to/your/vault`를 실제 보관함 경로로 바꾸세요.)

> 모바일(Android)에서는 파일 관리자의 "숨김 파일 표시"를 켜야
> `.obsidian` 폴더가 보입니다. `plugins` 폴더가 없으면 직접 만들면 됩니다.

### 2. Obsidian에서 활성화

1. Obsidian → **설정 → 커뮤니티 플러그인**
2. "제한 모드"가 켜져 있으면 끄기
3. "설치된 플러그인" 옆 새로고침 아이콘 클릭
4. **NAS Sync** 찾아서 토글 켜기

### 3. 페어링

1. **설정 → NAS Sync** (왼쪽 사이드바)
2. **Server URL**: `http://<NAS IP>:8080` — 예: `http://192.168.1.10:8080`
3. **Device name**: 예: `MacBook`
4. **Pairing code**: NAS의 `ONS_PAIRING_CODE` 값 붙여넣기
5. **Pair this device** 클릭

"Paired with NAS"가 뜨면 성공입니다. 페어링 코드는 성공 후 설정에서
자동으로 지워집니다.

**최초로 페어링한 기기**의 보관함 전체가 NAS에 업로드되고, 이후
페어링하는 기기는 첫 연결 때 그 보관함을 그대로 내려받습니다.

> ⚠ **두 번째 기기부터는 빈 보관함에 페어링하세요.** 서버에 이미 있는
> 경로와 겹치는 로컬 파일은 첫 동기화 때 서버 버전으로 덮어써져
> **기존 데이터가 사라질 수 있습니다.** 확실하지 않으면 먼저 백업하세요.

### 4. 동기화 확인

노트를 만들거나 수정해보세요. 5초(디바운스 윈도우) 안에:

```bash
# NAS에서
sudo docker compose logs --tail 30 obsidian-nas
# PUT /file/notes/foo.md 같은 줄이 보입니다
ls /volume1/docker/obsidian-nas/data/vault/
# 노트가 여기 나타납니다
```

### 문제 해결

- **"Pairing failed: HTTP 401"** → 페어링 코드가 틀렸거나 서버에
  `ONS_PAIRING_CODE`가 설정돼 있지 않습니다.
- **Obsidian에 플러그인이 안 보임** → `<보관함>/.obsidian/plugins/nas-sync/`에
  `manifest.json`과 `main.js`가 **둘 다** 있는지 확인하고 목록을
  새로고침하세요.
- **macOS에서 `ERR_ADDRESS_UNREACHABLE`** → macOS 15+의 로컬 네트워크
  권한 문제입니다. 시스템 설정 → 개인정보 보호 및 보안 → 로컬 네트워크에서
  Obsidian을 허용하고 앱을 재시작하세요.
- **페어링은 되는데 동기화가 안 됨** → Obsidian 개발자 콘솔
  (Cmd+Opt+I → Console)에서 `[nas-sync]` 로그를 확인하세요. NAS 서버
  로그도 함께.
- **WebSocket이 계속 재연결됨** → 서버 URL은 맞는데 `/sync`에 못 닿는
  상태입니다. 리버스 프록시 뒤라면 `Upgrade: websocket` 헤더를 전달하는지
  확인하세요.
