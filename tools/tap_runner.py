"""Attach serial_tap.js to G2CPS.Main.exe and stay attached until the process exits or a stop file appears."""
import os, sys, time
import frida

HERE = os.path.dirname(os.path.abspath(__file__))
STOP = os.path.join(HERE, 'tap.stop')
TARGET = 'G2CPS.Main.exe'

def find_pid():
    for p in frida.get_local_device().enumerate_processes():
        if p.name.lower() == TARGET.lower():
            return p.pid
    return None

pid = int(sys.argv[1]) if len(sys.argv) > 1 else find_pid()
if not pid:
    sys.exit(f'{TARGET} is not running')

session = frida.attach(pid)
done = []
session.on('detached', lambda reason, crash: (print('detached:', reason, flush=True), done.append(1)))
script = session.create_script(open(os.path.join(HERE, 'serial_tap.js'), encoding='utf-8').read())
script.on('message', lambda m, d: print('msg:', m.get('payload', m), flush=True))
script.load()
print(f'tapping pid {pid}', flush=True)

if os.path.exists(STOP):
    os.remove(STOP)
while not done and not os.path.exists(STOP):
    time.sleep(0.5)
if not done:
    script.unload()
    session.detach()
print('stopped', flush=True)
