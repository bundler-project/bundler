#!/bin/bash

ETH=10gp1
TC_DIR=/home/frank/bundler/qdisc/iproute2/tc
SPORT=28316
SIP=10.1.1.2

echo "=> remove qdisc"
sudo env TC_LIB_DIR=$TC_DIR tc qdisc del dev $ETH root

echo "=> remove module"
sudo rmmod sch_bundle_inbox

echo "=> make $1"
make $1 || exit

echo "=> insmod"
sudo insmod sch_bundle_inbox.ko || exit

echo "=> add root qdisc (prio)"
sudo tc qdisc add dev $ETH root handle 1: prio bands 3 || exit

echo "===> add qdisc child 1 (pfifo_fast)"
sudo tc qdisc add dev $ETH parent 1:1 pfifo_fast || exit
echo "===> filter out-of-band pkts to child 1"
sudo tc filter add dev $ETH parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip sport $SPORT 0xffff match ip src $SIP flowid 1:1 || exit

echo "===> add qdisc child 2 (bundle_inbox)"
sudo env TC_LIB_DIR=/home/frank/bundler/qdisc/iproute2/tc tc qdisc add dev 10gp1 parent 1:2 bundle_inbox rate 100mbit burst 1mbit limit 15mbit || exit
# echo "===> filter everything else to child 2"
# sudo tc filter add dev 10gp1 parent 1: protocol ip prio 2 u32 match ip dst 0.0.0.0/0 flowid 1:2

sudo env TC_LIB_DIR=/home/frank/bundler/qdisc/iproute2/tc tc -s qdisc show dev 10gp1
