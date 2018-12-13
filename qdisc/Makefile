TARGET = sch_bundle_inbox
objs := sch_bundle_inbox.o

obj-m := $(TARGET).o

all:
	make -C /lib/modules/$(shell uname -r)/build M=$(PWD) modules

clean:
	make -C /lib/modules/$(shell uname -r)/build M=$(PWD) clean
