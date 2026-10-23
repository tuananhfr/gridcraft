# Tương thích macro VBA / VBA compatibility

Bảng này mô tả phần triển khai hiện tại, không phải cam kết tương thích toàn bộ Excel.
Mức 1 là kết quả phân tích tĩnh; macro vẫn có thể gặp cú pháp hoặc thành viên chưa hỗ trợ khi chạy.

## Tiếng Việt

### Nơi chạy và mức tương thích

| Khả năng | Desktop | Web |
|---|---|---|
| Đọc VBA trong XLS/XLSM; xem module, thủ tục, lý do và dòng trong hộp Macro | Có | Có |
| Chạy Sub public trong standard module, không có tham số bắt buộc, mức 1 | Có | Có |
| Chạy qua nút Form Control được gán macro | Có | Có |
| UDF mức 1 trong công thức, sau khi đồng ý | Có | Có |
| DAO/AutoCAD giả lập trong bộ nhớ | Có | Có |
| Tự chạy macro khi mở tệp, sự kiện, ActiveX/UserForm | Không | Không |

| Mức trong hộp Macro | Ý nghĩa và hành vi |
|---|---|
| 1 | VBA/Excel hoặc COM thuộc tập giả lập; có thể chạy nếu đáp ứng dạng thủ tục ở trên. Không bảo đảm mọi thành viên đều được triển khai. |
| 2 | Phụ thuộc Windows/API hoặc truy cập hệ thống/tệp bị phát hiện; từ chối chạy. |
| 3 | COM/automation ngoài tập giả lập; từ chối chạy, không khởi động ứng dụng ngoài. |
| 4 | ActiveX/UserForm hoặc sự kiện điều khiển không được hỗ trợ; từ chối chạy. |

Phân loại lan truyền qua các lời gọi được phân tích. Hộp Macro hiển thị lý do và vị trí;
việc có một thủ tục không hỗ trợ không tự động cấm mọi thủ tục an toàn trong cùng tệp.
Lỗi biên dịch được cô lập theo module: các module khác vẫn chạy; lời gọi vào module lỗi báo lỗi
biên dịch và vị trí của module đó. Hộp Macro hiển thị lỗi trên mọi thủ tục của module và không cho chạy chúng.

### Đồng ý, hoàn tác và giới hạn

Người dùng phải đồng ý cho từng phiên tài liệu; quyền không được lưu vào tệp. Mở tệp không chạy VBA.
Macro chạy trên bản sao cô lập; thành công được ghi thành một lần Undo. Lỗi, Dừng hoặc hết giới hạn
hủy thay đổi của macro; chỉnh sửa xen giữa không bị ghi đè. Nhật ký có trạng thái, bước chạy, số ô thay đổi và vị trí lỗi.
Macro không được tự đọc/ghi ổ đĩa, truy cập mạng hoặc điều khiển hệ điều hành. Chọn MDB là cấp dữ liệu tường minh;
AutoCAD chỉ tạo DXF để giao diện xuất, không kết nối AutoCAD thật.

Mặc định: desktop 30 giây, web 5 giây; 1.000.000 bước, 32 MiB bộ nhớ interpreter.
Có giới hạn độ sâu lời gọi, mảng, chuỗi và vùng ô; giới hạn không phải tổng RAM của ứng dụng.
Nút Dừng hoạt động qua thực thi hợp tác. Thời gian chờ picker MDB được loại khỏi thời hạn chạy.
MsgBox/InputBox tạm dừng chờ trả lời trong giao diện; chạy headless dùng mặc định.
`GetOpenFilename` hiện luôn trả False, kể cả sau prompt. `Workbook.Path` trả chuỗi rỗng.
`AppActivate`, `ChDir`, `ChDrive` là no-op có nhật ký; `CurDir` trả chuỗi rỗng, không đọc thư mục thật.

### Ngôn ngữ và hàm có sẵn

| Nhóm | Tập đã triển khai |
|---|---|
| Khai báo/lời gọi | Sub, Function, Property Get/Let/Set; Dim, Const, Static; kiểu scalar/Variant; ByVal/ByRef, Optional, ParamArray; Call, Let, Set; đối số đặt tên và bỏ trống |
| Điều khiển | If/ElseIf/Else, Select Case (giá trị, To, Is), For/Next/Step, For Each, Do/Loop While/Until, While/Wend, With, Exit, nhãn/GoTo |
| Lỗi/mảng | On Error GoTo/Resume Next/GoTo 0, Resume; Err; mảng nhiều chiều, ReDim/Preserve, Erase; Option Explicit/Base/Compare |
| Khác | Randomize, Debug.Print, Stop; biểu thức số, chuỗi, so sánh và logic; nối dòng, dấu hai chấm, tham chiếu bang |

Các hàm nội tại sau có triển khai (ngữ nghĩa/định dạng chỉ là tập con VBA):

`len, left, right, mid, instr, instrrev, replace, split, join, ucase, lcase, trim, ltrim, rtrim, space, string, strcomp, chr, chrw, asc, ascw, format, strreverse, abs, atn, sin, cos, tan, exp, log, sqr, int, fix, round, rnd, sgn, now, date, time, timer, dateserial, dateadd, datediff, year, month, day, hour, minute, second, weekday, lbound, ubound, array, isarray, isnumeric, isempty, isnull, isobject, isdate, ismissing, typename, vartype, cint, clng, clnglng, cdbl, cstr, cbool, cdate, cvar, csng, ccur, cbyte, cverr, val, iif, choose, switch`.

### Đối tượng Excel

| Đối tượng | Thành viên có triển khai; giới hạn đáng chú ý |
|---|---|
| Application/global | ActiveWorkbook, ThisWorkbook, ActiveSheet, ActiveCell, Selection, Sheets/Worksheets, Range/Cells/Rows/Columns, WorksheetFunction, Calculate, Union, Intersect, ScreenUpdating, EnableEvents, DisplayAlerts, StatusBar, Calculation, GetOpenFilename, InputBox, Wait, Evaluate |
| Workbook | Name (đọc), Path (rỗng), Sheets/Worksheets; Save bị từ chối |
| Worksheets | Item/chỉ số hoặc tên, Count; code name của sheet được phân giải |
| Worksheet | Name, Visible (kể cả VeryHidden), Range, Cells, Rows, Columns, UsedRange, Activate/Select, Calculate, PageSetup.PrintArea, Protect/Unprotect (không mật khẩu) |
| Range | Item/Cells/Range, Value/Value2, Formula/FormulaLocal/FormulaR1C1, Text (đọc), Offset, Resize, Row/Column, Count/CountLarge, Rows/Columns, Address, End, Find, Clear/ClearContents, Copy, PasteSpecial, Merge/UnMerge, Select/Activate, NumberFormat, Font, Interior, CopyFromRecordset, AutoFill |
| Định dạng | Font.Bold/Color, Interior.Color/ColorIndex; AutoFit hiện là no-op |
| WorksheetFunction | Các hàm mà bộ tính toán GridCraft nhận biết; tên có `_` có thể ánh xạ sang dấu chấm; đối số đặt tên Arg1…Arg255 |

Không có toàn bộ object model Excel: chẳng hạn các thuộc tính PageSetup khác, ExportAsFixedFormat,
Application.Interactive chưa được host triển khai.
`[A1]`, `[A1:B5]`, `[Sheet1!A1]`, `[MyName]` gọi `Application.Evaluate`: tham chiếu trả về Range
(hỗ trợ `.Value`, gán giá trị mặc định và Set), biểu thức khác trả về giá trị tính toán.
UDF chỉ đọc, ghi nhận ô phụ thuộc và từ chối ghi qua Range trả về.
Cũng hỗ trợ `sheet.[A1]` và `.[A1]` trong With. Code name của worksheet được đọc từ XLS/XLSM,
giữ lại khi lưu OOXML và liên kết tới sheet; lời gọi thành viên Excel không bị nhầm với thủ tục VBA private.
Clear/ClearContents xử lý ô đã lưu trong vùng thưa thay vì liệt kê toàn bộ ô trống; vẫn giới hạn số ô thực sự xử lý.
So sánh Empty/chuỗi và Variant số/chuỗi hỗ trợ vòng lặp tìm ô trống.
AutoFill dùng bộ điền hiện có với Type mặc định, Copy, Formats và Values; đích phải chứa nguồn,
cùng worksheet và mở rộng theo một trục. Các Type khác bị từ chối; có giới hạn ô/bộ nhớ.
Range cũng nhận defined name trỏ tới vùng; PageSetup.PrintArea đọc/ghi/xóa một vùng in thông qua lệnh engine,
được hoàn tác cùng macro và không truy cập máy in hay ổ đĩa.
Các đối tượng Excel trả về Application để tiếp tục chuỗi thành viên như worksheet.Application.ActiveSheet. Copy/PasteSpecial/Find chỉ có những tùy chọn được host xử lý;
không suy ra hỗ trợ mọi đối số Excel từ việc tên phương thức xuất hiện trong bảng.

### COM giả lập

DAO: nhận `DAO`, `DBEngine`, `OpenDatabase` và các ProgID DAO DBEngine được host nhận biết;
Workspaces, OpenDatabase, Database.Close/TableDefs/OpenRecordset; TableDefs.Item/Count, TableDef.Name;
Recordset.EOF/BOF/RecordCount/Fields/Item, MoveFirst/Last/Next/Previous, Close;
Fields.Item/Count, Field.Name/Value, cú pháp `rs!Field`, Range.CopyFromRecordset.
Dữ liệu MDB do người dùng chọn được đọc và cache trong bộ nhớ. Không Edit/AddNew/Update, ghi Field hay Database.Execute.

Truy vấn nhận tên bảng trực tiếp hoặc tập con:

```text
SELECT * | column [, column ...] FROM table
  [WHERE column (= | <> | < | > | <= | >=) literal
    [AND ... | OR ...]]
  [ORDER BY column [ASC | DESC] [, column [ASC | DESC] ...]] [;]
```

Tên có khoảng trắng dùng ngoặc vuông; literal gồm chuỗi/số được tokenizer nhận biết.
AND nhóm trước OR. Không JOIN, hàm tổng hợp, GROUP BY, alias, LIKE, dấu ngoặc nhóm điều kiện,
truy vấn ghi hay nhiều câu SQL. Tối đa 256 cột chiếu; còn có giới hạn dữ liệu và số ô sao chép.

AutoCAD: CreateObject/GetObject với AutoCAD.Application; Visible, Caption, ActiveDocument,
Documents.Add/Item/Count; Document.ModelSpace/Layers/ActiveLayer; Layers.Add/Item/Count;
Layer.Name/Color; ModelSpace.AddLine/AddPoint/AddCircle/AddArc/AddText/AddMText/
AddLightWeightPolyline/AddPolyline; Entity.Layer/Color/Rotate. Tọa độ truyền bằng mảng.
ZoomAll/ZoomExtents/Update/Regen là no-op trên các đối tượng tương ứng; hình học được xuất thành DXF.
Không có COM thật, tiến trình AutoCAD, UI CAD hay thành viên tùy ý.

### UDF trong công thức

Hàm public đủ điều kiện UDF ở standard module, mức 1, được gọi qua tên thường hoặc `_xludf.`.
Cần đồng ý trong phiên; tham chiếu Range chỉ đọc và được theo dõi dependency.
Không ghi ô, prompt, file picker, COM hay thao tác UI. Giới hạn mỗi lần gọi: 10.000 bước, 32 mức gọi,
4.096 phần tử/vùng ô, chuỗi 64 KiB, bộ nhớ 1 MiB, thời hạn 50 ms; không được tạm dừng.
Lỗi runtime/giới hạn trả lỗi ô và có chẩn đoán module/dòng trong Error Checking.

### Những phần không hỗ trợ và lý do

| Phần | Lý do |
|---|---|
| DLL/Declare, Shell, SendKeys, Environ, file I/O, network | Không cấp quyền hệ thống cho VBA |
| COM khác DAO/AutoCAD giả lập, ADODB, ETABS automation | Không có cầu nối COM thật |
| ActiveX, UserForm, sự kiện tự động, class tùy ý | Chưa có runtime/giao diện tương ứng; không tự kích hoạt mã |
| Workbook.Save/Open, xuất PDF theo đường dẫn VBA | Host không cấp truy cập ổ đĩa |
| Thành viên Excel/cú pháp ngoài tập trên | Chưa triển khai; mức 1 không phải chứng nhận chạy đúng |

### Nguồn triển khai và nghiệm thu

Danh sách lấy từ [parser](../crates/vbarun/src/parser.rs), [intrinsics](../crates/vbarun/src/intrinsic.rs),
[phân loại](../crates/vba/src/classify.rs), [host Excel](../crates/engine/src/macro_host/host.rs),
[COM giả lập](../crates/engine/src/macro_host/emulation.rs), [SQL](../crates/engine/src/macro_host/emulation/query.rs),
[giới hạn/transaction](../crates/engine/src/macro_host/mod.rs) và [UDF](../crates/engine/src/macro_host/udf.rs).

[Acceptance test](../crates/engine/tests/vba_acceptance.rs) là test ignored, opt-in qua
`GRIDCRAFT_VBA_SAMPLE`, tùy chọn `GRIDCRAFT_VBA_MDB`. Không đưa tệp khách hàng vào repo.
Test chỉ báo số lượng, trạng thái, bước, số ô thay đổi, prompt và lỗi/vị trí; hoàn thành chạy
không chứng minh kết quả số khớp Excel. Cần đối chiếu riêng bằng Excel thật trước khi tuyên bố đạt nghiệm thu.

## English

Desktop and web share the sandboxed VBA interpreter and Excel host. Public level-1 standard-module Subs
without required arguments can run with session consent; eligible read-only UDFs can calculate in formulas.
Level 2 (system dependencies), level 3 (unsupported automation) and level 4 (unsupported UI/control events) are refused.
Level 1 is static classification, not a guarantee of runtime compatibility or numerical parity.
Compile errors are isolated per module: other modules can run, while calls into a failed module raise
its compile error with module/line. Macro analysis shows the error on its procedures and disables Run.
Square brackets (`[A1]`, `[A1:B5]`, `[Sheet1!A1]`, `[MyName]`) call Application.Evaluate;
references return Range objects supporting members, default-value assignment and Set. Other expressions
return calculated values. Read-only UDF evaluation tracks precedents and rejects writes through returned ranges.
Worksheet-qualified brackets (`sheet.[A1]`, `.[A1]` in With) use that sheet's context. Worksheet code names
are imported from XLS/XLSM, preserved in OOXML and bound to sheet objects; built-in Excel members
are delegated to the host while declared VBA procedure visibility is preserved.
Clear/ClearContents budget stored cells in sparse ranges without expanding blank worksheet space.
Empty/string and numeric/string Variant comparisons support loops that stop at blank cells.
AutoFill delegates default, Copy, Formats and Values modes to the existing fill engine, with cell/memory
budgets and a same-sheet destination containing and extending the source along one axis. Other types are refused.
Range accepts defined names referring to ranges. PageSetup.PrintArea reads, sets and clears a single print
area through engine commands, within the macro's undo transaction and without printer or disk access.
Excel objects expose Application for member chains such as worksheet.Application.ActiveSheet.

There is no automatic open/event execution or unrestricted disk, network or OS access. Successful macro edits
form one undo transaction; errors/cancellation roll back. Default deadlines are 30 seconds desktop and 5 seconds web,
with step/memory/depth limits. DAO reads explicitly selected MDB bytes with the SELECT subset above;
AutoCAD calls build DXF in memory, without real COM. GetOpenFilename still returns False.
UDFs cannot write, prompt or suspend and have tighter per-call limits. The Vietnamese tables above enumerate
implemented language features, built-ins, host members and limitations, with direct implementation links.
The opt-in acceptance harness reports outcomes, not proof of Excel-equivalent customer results.
