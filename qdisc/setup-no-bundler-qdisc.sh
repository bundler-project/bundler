#!/bin/bash

ETH=10gp1
LIMIT=$1

echo "=> remove qdisc"
sudo tc qdisc del dev $ETH root

echo "=> remove module"
sudo rmmod sch_bundle_inbox

echo "=> add root qdisc (bfifo) with queue size = $LIMIT"
sudo tc qdisc add dev $ETH root bfifo limit $LIMIT || exit

sudo tc -s qdisc show dev 10gp1
