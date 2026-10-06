# Record with: vhs docs/media/refactrail-lint-fix.tape (from the repository root)
Output docs/media/refactrail-lint-fix.gif
Set FontSize 18
Set Width 1100
Set Height 620
Set Padding 20
Set TypingSpeed 45ms
Set Shell bash

Hide
Type "rm -rf /tmp/rt-demo && mkdir -p /tmp/rt-demo && cp examples/lint_fix_demo.py /tmp/rt-demo/demo.py && cd /tmp/rt-demo && clear"
Enter
Show
Type "refactrail lint demo.py --select E4,E7,F"
Enter
Sleep 3s
Type "refactrail lint demo.py --select E4,E7,F --diff"
Enter
Sleep 4s
Type "refactrail lint demo.py --select E4,E7,F --fix"
Enter
Sleep 3s
Type "refactrail lint demo.py --select E4,E7,F"
Enter
Sleep 3s
