#!/bin/sh

# git-remote-k8s, a remote helper for git targeting a kubernetes cluster
# 
# Copyright (C) 2023  James Andariese
# 
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU Affero General Public License as
# published by the Free Software Foundation, either version 3 of the
# License, or (at your option) any later version.
# 
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Affero General Public License for more details.
# 
# You should have received a copy of the GNU Affero General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.
set -e

export IMAGE=alpine/git:latest

export CONTEXT="$(echo "${2#*://}" | cut -d / -f 1)"
export NS="$(echo "${2#*://}" | cut -d / -f 2)"
export REPO="$(echo "${2#*://}" | cut -d / -f 3)"
export RUNID="$(dd if=/dev/random bs=600 count=1 status=none | base64 | tr -dc a-z0-9 | cut -c 1-6)"

while read -r cmd arg rest; do
    case "$cmd" in
        "capabilities")
            printf 'connect\n\n'
        ;;
        "connect")
            case "$arg" in
              git-receive-pack) SUBCOMMAND="git receive-pack .";;
              git-send-pack) SUBCOMMAND="git send-pack .";;
              git-upload-pack) SUBCOMMAND="git upload-pack .";;
              git-upload-archive) SUBCOMMAND="git upload-archive .";;
              *)
                1>&2 echo "invalid subcommand in connect $arg"
                exit 1
              ;;
            esac
            1>&2 echo "running $arg"
            break
        ;;
    esac
done

export COMMAND="
[ -f HEAD ] || git init --bare
read
echo
$SUBCOMMAND
"
# if you named your pod FILTERME_HFOIQJF, I apologize

echo '
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: ${REPO}
  namespace: ${NS}
spec:
  accessModes:
  - ReadWriteOnce
  resources:
    requests:
      storage: 1Gi
  storageClassName: ${GIT_REMOTE_K8S_STORAGECLASS-FILTERME_HFOIQJF}
  volumeMode: Filesystem
---
apiVersion: batch/v1
kind: Job
metadata:
  name: ${REPO}-gitc${RUNID}
  namespace: ${NS}
  labels:
    git.kn8v.com/runid: ${RUNID}
spec:
  template:
    spec:
      containers:
      - name: git-connector
        image: ${IMAGE}
        stdin: true
        stdinOnce: true
        command:
        - sh
        - -c
        - ${COMMAND}
        workingDir: /repo
        volumeMounts:
        - name: repo
          mountPath: /repo
      volumes:
      - name: repo
        persistentVolumeClaim:
          claimName: ${REPO}
      restartPolicy: Never
' | grep -v FILTERME_HFOIQJF | yq ea '(.. | select(tag == "!!str")) |= envsubst' | kubectl --context "$CONTEXT" apply -f - 1>&2

KILLLOGS=:

finalize() {
  kubectl --context "$CONTEXT" delete job -n "$NS" "${REPO}-gitc${RUNID}" 1>&2
  $KILLLOGS
  exit  # must exit for INT and TERM.
}
trap finalize INT TERM

kubectl --context "$CONTEXT" wait job "${REPO}-gitc${RUNID}" --for jsonpath=.status.ready=1 1>&2
(echo;cat) | kubectl --context "$CONTEXT" attach -i -q -n "$NS" "job/${REPO}-gitc${RUNID}"

# also finalize on exit
finalize
