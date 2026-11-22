# CI에서 시연용 앱 만들기

저장소 루트는 현재 `app/` 디렉터리의 내용으로 구성한다. `.github/`, `.node-version`, `rust-toolchain.toml`, `package-lock.json`, `src-tauri/Cargo.lock`을 함께 커밋한다. 상위 연구 폴더 전체를 저장소 루트로 사용할 경우 workflow 위치와 working-directory를 먼저 조정해야 한다.

1. 변경을 GitHub에 push한다. 수동 실행 workflow는 기본 브랜치에도 있어야 Actions 실행 메뉴에 나타난다.
2. Actions → **Package desktop preview** → Run workflow에서 시연할 브랜치를 선택한다.
3. 각 OS 작업이 성공하면 Artifacts에서 해당 commit의 다운로드를 선택한다.
4. macOS: 다운로드를 풀고 내부 `Branchdesk-macos-arm64.zip`을 풀면 `.app`이 나온다. Windows: `Branchdesk-windows-x64-setup.exe`를 사용한다.
5. Actions artifact의 `manifest.json`/`SHA256SUMS`, 또는 Release의 `manifest-<platform>.json`/`SHA256SUMS-<platform>.txt`에서 commit/version과 체크섬을 기록해 시연 빌드를 식별한다.

이 workflow는 빌드·패키징 전용이다. 테스트를 몰래 실행하거나 검증 통과로 표시하지 않는다. 별도 **Check desktop** workflow가 기능 검사를 수행한다. 2026-11-22 첫 CI에서 macOS 패키징은 성공했지만 backend 검사는 578 통과·15 실패·8 제외였다. TLS fixture 시작, provider write 기대값, Git lock/recovery 실패는 후속 수정 대상이다. Windows에서 누락된 icon.ico를 발견하여 기존 앱 아이콘으로 생성해 추가했다.

코드서명 인증서·공증 자격증명은 연결하지 않았다. macOS의 정상 Gatekeeper 배포 수용 및 Windows 신뢰된 서명은 이 산출물로 보장하지 않는다. 두 OS 패키징 성공 후 GitHub 프리뷰 Release를 자동 게시한다. 자동 업데이트는 제공하지 않는다. 서명 자격증명은 향후 보호된 GitHub Environment에 설정하며 저장소 파일에 넣지 않는다.

로컬 빌드는 사용자 지시로 실행하지 않는다. 이후 GitHub Actions에서 macOS ARM64와 Windows x64 패키징 성공 기록이 생겼다. 아래 commit별 범위를 확인하고, 이를 기능·복구 시나리오 검증 결과와 구분한다.

근거: [Tauri GitHub 배포 안내](https://v2.tauri.app/distribute/pipelines/github/), [GitHub artifact action](https://github.com/actions/upload-artifact).

## Authorized preview release

Package desktop preview now creates a draft prerelease, attaches both platform packages and platform-qualified manifests/checksums, and publishes only after both builds succeed. Failed runs leave a draft. Release tags use v0.1.0-preview.RUN_NUMBER. Production signing/notarization is still not configured.


## 확인된 배포 범위 — 2026-11-22

| 대상 | 소스 commit | 확인된 상태 | 포함 범위 |
| --- | --- | --- | --- |
| 공개 preview.4 | `e659027` | 두 OS 패키징 성공, 공개됨 | Gitflow 설정·초기화·Start까지. Finish 실행 및 최근 사이드바 제외 |
| 공개 preview.5 / run 44987167737 | `80a6436` | 두 OS 패키징 및 publish 성공, 공개됨 | Finish 병합·태그·삭제, 충돌 후 재진입, 기존 태그 확인 후 재개, 최종 정리 검증 |
| 공개 preview.6 / run 44987785549 | `0f49aee` | 두 OS 패키징 및 publish 성공, 공개됨 | Gitflow/Worktree 사이드바, 공통 헤더·조회 제어, Finish 진행 기록, 설정에서 Gitflow 열기 |
| main 후속 소스 | `3c1392b`부터 `ebd14f1`까지 | 위 빌드에 미포함 | Gitflow 설정 작업 제어, WIP 열기·첫 조회·결과 유지, Gitflow 폴더 탐색·명령 팔레트 |

상태는 위 시점의 기록이다. preview.5의 두 플랫폼 패키징과 publish 성공을 확인했다. 이 결과는 후속 main 변경의 컴파일 또는 기능 검증을 증명하지 않는다.

- [공개 preview.4](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.4)
- [공개 preview.5](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.5)
- [preview.5 빌드 실행](https://github.com/Polar-Little-Lab/reverse-engineering-test/actions/runs/44987167737)

### 아직 증명하지 않은 항목

- 실제 설치·실행 및 새 Gitflow/Worktree 흐름의 macOS/Windows 동작.
- 취소, 충돌 해결, 앱 종료, 디스크 부족과 중간 작업 복구 시나리오.
- 배포 바이너리 내부의 개발 전용 도구 미포함 여부 직접 검사.
- 기존 backend CI 실패의 수정 및 재실행. 패키징 workflow는 이 테스트들을 실행하지 않는다.
- 프로덕션 서명·공증·자동 업데이트.

preview.6의 macOS 업로드 확인: `Branchdesk-macos-arm64.zip` 17,123,211 bytes와 플랫폼 manifest/checksum. [동일 실행 추적](https://github.com/Polar-Little-Lab/reverse-engineering-test/actions/runs/44987785549). 이후 Windows 패키징 및 publish 성공까지 확인했다. [공개 preview.6](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.6).

## preview.7 compilation failure correction

Run `44988568984` at `9dde43b` failed on both platforms before native compilation: `IntegrationsPreferencesPanel.tsx` TS1005, missing closing JSX expression brace in the repository options prop. Neither platform package was produced by this run and publish was skipped. Corrected that expression and expanded JSX formatting for review. A new source build is required; this is not a runtime verification result.

## preview.8 published

Run `44988703609` at `dafa0dfcde0a19da3711a19230e053cf5a8013b7` completed successfully on macOS and Windows and published [preview.8](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.8). Both platform packages, manifests and checksums are present; release is not a draft. This resolves the preview.7 compilation failure. It includes Integrations credential controls, but not the subsequent Launchpad handoff or LFS implementation. Packaging success does not verify runtime scenarios.

## preview.9 published

Run `44989659498` at `4cdc64e2551466f476edab84d38716fb8074291d` completed both platform builds and publish successfully. [preview.9](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.9) is public, not a draft. It adds single-object LFS download and the earlier Launchpad handoff. It does not include later transfer cancellation, cache refresh or upload changes. No runtime LFS transfer acceptance is established by packaging success.

## preview.10 build and publish succeeded

Run `44990161587` at `52a81fb00281b404032652b1762ffc63e989037d` completed macOS, Windows and publish jobs successfully. This batch includes LFS transfer cancellation, cache refresh and single-object upload/verification. Subsequent durable records, remote observations, repository LFS menu and current-index inventory are not part of that source. Packaging success is not runtime transfer or crash-recovery acceptance.

## preview.11 published

Run `44990869011` at `dbaf66546c9af5511d0a572d35baf41a821f0071` completed successfully and published [preview.11](https://github.com/Polar-Little-Lab/reverse-engineering-test/releases/tag/v0.1.0-preview.11). Release assets include macOS arm64 ZIP, Windows x64 installer, both manifests and checksums. This includes durable upload records, remote observations, the repository LFS menu and paged index inventory. Later whole-index catalog, batch transfer and checkout preview are not included. Packaging is not runtime acceptance.
