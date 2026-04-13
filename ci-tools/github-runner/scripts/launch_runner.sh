# Licensed under the Apache-2.0 license

#!/bin/bash

function set_guest_attr()
{
    curl -X PUT --data "$2" "http://metadata.google.internal/computeMetadata/v1/instance/guest-attributes/caliptra-github-ci/$1" -H "Metadata-Flavor: Google"
}

set -x
echo "Starting GitHub Runner launch script"
set_guest_attr "runner-state" "STARTING"

cd /home/runner
su runner -l -c "export"

echo "Executing run.sh"
# Run the runner and capture the exit code
su runner -l -c "/home/runner/actions-runner/run.sh --jitconfig '${JITCONFIG}'"
exit_code=$?

echo "run.sh exited with code $exit_code"
if [ $exit_code -eq 0 ]; then
    set_guest_attr "runner-state" "SUCCESS"
else
    set_guest_attr "runner-state" "FAILURE"
    set_guest_attr "runner-error" "exit-code-$exit_code"
fi

# Give a small window for the attribute to persist before shutdown
sleep 5
shutdown -h now