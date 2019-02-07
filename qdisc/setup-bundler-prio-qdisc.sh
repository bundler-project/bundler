#!/bin/bash

ETH=10gp1
TC_DIR=/home/`whoami`/bundler/qdisc/iproute2/tc
INBOX_SPORT=28316
INBOX_SIP=10.1.1.2
LIMIT=$1

echo "Script deprecated. This functionality is now included in the inbox application."
exit 1

echo "=> remove qdisc"
sudo env TC_LIB_DIR=$TC_DIR tc qdisc del dev $ETH root

echo "=> remove module"
sudo rmmod sch_bundle_inbox

echo "=> make prio"
make QTYPE=prio || exit

echo "=> insmod"
sudo insmod sch_bundle_inbox.ko || exit

echo "=> add root qdisc (prio)"
sudo tc qdisc add dev $ETH root handle 1: prio bands 3 || exit

echo "===> add qdisc child 1 (pfifo_fast)"
sudo tc qdisc add dev $ETH parent 1:1 pfifo_fast || exit
echo "===> filter out-of-band pkts to child 1"
sudo tc filter add dev $ETH parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport $INBOX_SPORT 0xffff match ip src $INBOX_SIP flowid 1:1 || exit

echo "===> add qdisc child 2 (bundle_inbox) with queue size $LIMIT"
sudo env TC_LIB_DIR=$TC_DIR tc qdisc add dev 10gp1 parent 1:2 bundle_inbox rate 100mbit burst 1mbit limit $LIMIT || exit

#echo "==> add prio 1 to interactive traffic (ports 4096-8191)"
# 4096 + 0xf000 mask = portrange 4096-8191
#sudo tc filter add dev $ETH parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport 4096 0xf000 match ip src $SRC_SIP flowid 1:2 || exit

# echo "===> filter everything else to child 2"
# sudo tc filter add dev 10gp1 parent 1: protocol ip prio 2 u32 match ip dst 0.0.0.0/0 flowid 1:2

sudo env TC_LIB_DIR=$TC_DIR tc -s qdisc show dev 10gp1
