#!/bin/bash

sudo apt-get install libdb-dev

git submodule update --init --recursive

cd iproute2
./configure
cd ..

cp q_bundle_inbox.c iproute2/tc/

cd iproute2
make TCSO=q_bundle_inbox.so

echo "TC_LIB_DIR=$pwd/tc"
