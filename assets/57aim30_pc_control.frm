VERSION 5.00
Object = "{648A5603-2C6E-101B-82B6-000000000014}#1.1#0"; "mscomm32.ocx"
Begin VB.Form YZ_AIM_v2_61 
   BackColor       =   &H8000000A&
   Caption         =   "YZ_AIM_v2_61"
   ClientHeight    =   10095
   ClientLeft      =   630
   ClientTop       =   345
   ClientWidth     =   14190
   FillStyle       =   0  'Solid
   LinkTopic       =   "Form1"
   ScaleHeight     =   673
   ScaleMode       =   3  'Pixel
   ScaleWidth      =   946
   Begin VB.Frame Frame_modbus_send_display 
      Caption         =   "发送数据"
      Height          =   975
      Left            =   0
      TabIndex        =   118
      Top             =   9120
      Width           =   10575
      Begin VB.TextBox Text_send_desplay 
         Height          =   615
         Left            =   120
         MultiLine       =   -1  'True
         ScrollBars      =   2  'Vertical
         TabIndex        =   120
         Text            =   "test1.frx":0000
         Top             =   240
         Width           =   10335
      End
   End
   Begin VB.Frame Frame_modbus_read 
      Caption         =   "modbus读取"
      Height          =   2175
      Left            =   5520
      TabIndex        =   62
      Top             =   6840
      Width           =   2415
      Begin VB.CheckBox Check_search_read_modbus_add 
         Caption         =   "地址扫描"
         Height          =   255
         Left            =   1200
         TabIndex        =   113
         Top             =   720
         Width           =   1095
      End
      Begin VB.CheckBox Check_begin_read_modbus 
         Caption         =   "开始读取"
         Height          =   255
         Left            =   120
         TabIndex        =   69
         Top             =   720
         Value           =   1  'Checked
         Width           =   1095
      End
      Begin VB.TextBox Text_device_add 
         Height          =   270
         Left            =   1440
         TabIndex        =   68
         Text            =   "1"
         Top             =   360
         Width           =   735
      End
      Begin VB.Frame Frame6 
         Caption         =   "读取周期"
         Height          =   975
         Left            =   240
         TabIndex        =   63
         Top             =   1080
         Width           =   1935
         Begin VB.OptionButton Option_100ms 
            Caption         =   "0.05s"
            Height          =   180
            Left            =   120
            TabIndex        =   70
            Top             =   360
            Value           =   -1  'True
            Width           =   855
         End
         Begin VB.OptionButton Option_1s 
            Caption         =   "0.02s"
            Height          =   180
            Left            =   960
            TabIndex        =   66
            Top             =   600
            Width           =   855
         End
         Begin VB.OptionButton Option_500ms 
            Caption         =   "0.2s"
            Height          =   180
            Left            =   120
            TabIndex        =   65
            Top             =   600
            Width           =   735
         End
         Begin VB.OptionButton Option_200ms 
            Caption         =   "0.1s"
            Height          =   225
            Left            =   960
            TabIndex        =   64
            Top             =   360
            Width           =   735
         End
      End
      Begin VB.Label Label15 
         Caption         =   "读取设备地址:"
         Height          =   255
         Left            =   120
         TabIndex        =   67
         Top             =   360
         Width           =   1455
      End
   End
   Begin VB.Frame Frame_modbus_write 
      Caption         =   "modbus发送"
      Height          =   2775
      Left            =   10680
      TabIndex        =   55
      Top             =   7320
      Width           =   3495
      Begin VB.CommandButton Command_modbus_send 
         Caption         =   "发送"
         Height          =   375
         Left            =   2760
         TabIndex        =   61
         Top             =   1320
         Width           =   615
      End
      Begin VB.TextBox Text_send_data 
         Height          =   270
         Left            =   1440
         TabIndex        =   59
         Text            =   "1"
         Top             =   840
         Width           =   1575
      End
      Begin VB.ComboBox Combo_send_type 
         Height          =   300
         IMEMode         =   1  'ON
         ItemData        =   "test1.frx":0006
         Left            =   1440
         List            =   "test1.frx":004C
         Style           =   2  'Dropdown List
         TabIndex        =   56
         Top             =   360
         Width           =   1815
      End
      Begin VB.Label Label_send_note 
         Caption         =   "参数含义:"
         Height          =   1200
         Left            =   240
         TabIndex        =   60
         Top             =   1440
         Width           =   2415
      End
      Begin VB.Label Label20 
         Caption         =   "参数数据:"
         Height          =   255
         Left            =   240
         TabIndex        =   58
         Top             =   960
         Width           =   975
      End
      Begin VB.Label Label14 
         Caption         =   "参数类型:"
         Height          =   255
         Left            =   240
         TabIndex        =   57
         Top             =   480
         Width           =   975
      End
   End
   Begin VB.Frame Frame3 
      Caption         =   "modbus控制参数"
      Height          =   7335
      Left            =   10680
      TabIndex        =   43
      Top             =   0
      Width           =   3495
      Begin VB.CommandButton para_save 
         Caption         =   "参数保存"
         Height          =   375
         Left            =   2280
         TabIndex        =   117
         Top             =   6120
         Width           =   1095
      End
      Begin VB.Label Label_position_forward 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   112
         Top             =   5760
         Width           =   615
      End
      Begin VB.Label Label4 
         Caption         =   "特殊功能:"
         Height          =   255
         Left            =   240
         TabIndex        =   111
         Top             =   5760
         Width           =   1335
      End
      Begin VB.Label Label_speed_filter 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   110
         Top             =   5400
         Width           =   615
      End
      Begin VB.Label Label2 
         Caption         =   "静态最大输出:"
         Height          =   255
         Left            =   240
         TabIndex        =   109
         Top             =   5400
         Width           =   1215
      End
      Begin VB.Label Label_modbus_PU 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   108
         Top             =   1080
         Width           =   1215
      End
      Begin VB.Label Label_modbus_PU_total 
         Caption         =   "0"
         ForeColor       =   &H00FF0000&
         Height          =   255
         Left            =   1560
         TabIndex        =   107
         Top             =   1440
         Width           =   975
      End
      Begin VB.Label Label1 
         Caption         =   "PU(总步数)"
         Height          =   255
         Left            =   240
         TabIndex        =   106
         Top             =   1440
         Width           =   1575
      End
      Begin VB.Label Label_DIR_polarity_explain 
         Caption         =   "正逻辑"
         Height          =   255
         Left            =   2280
         TabIndex        =   103
         Top             =   4320
         Width           =   1095
      End
      Begin VB.Label Label30 
         Caption         =   "V/KRPM"
         Height          =   255
         Left            =   2280
         TabIndex        =   102
         Top             =   3960
         Width           =   855
      End
      Begin VB.Label LabelE_Gear_denominator 
         Caption         =   "1"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   2160
         TabIndex        =   101
         Top             =   4680
         Width           =   615
      End
      Begin VB.Label Label29 
         Caption         =   "/"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   2040
         TabIndex        =   100
         Top             =   4680
         Width           =   135
      End
      Begin VB.Label Label_E_Gear_numerator 
         Alignment       =   1  'Right Justify
         Caption         =   "1"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   99
         Top             =   4680
         Width           =   495
      End
      Begin VB.Label Label_DIR_polarity 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   98
         Top             =   4320
         Width           =   735
      End
      Begin VB.Label Label_speed_feedforward 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   97
         Top             =   3960
         Width           =   615
      End
      Begin VB.Label Label28 
         Caption         =   "电子齿轮比:"
         Height          =   255
         Left            =   240
         TabIndex        =   96
         Top             =   4680
         Width           =   1095
      End
      Begin VB.Label Label26 
         Caption         =   "DIR极性"
         Height          =   255
         Left            =   240
         TabIndex        =   95
         Top             =   4320
         Width           =   1095
      End
      Begin VB.Label Label24 
         Caption         =   "速度前馈:"
         Height          =   255
         Left            =   240
         TabIndex        =   94
         Top             =   3960
         Width           =   1095
      End
      Begin VB.Label Label_position_Kp 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   92
         Top             =   3600
         Width           =   615
      End
      Begin VB.Label Label22 
         Caption         =   "位置环Kp:"
         Height          =   255
         Left            =   240
         TabIndex        =   91
         Top             =   3600
         Width           =   855
      End
      Begin VB.Label Label_modbus_save_flag_explain 
         Caption         =   "未保存"
         Height          =   255
         Left            =   2160
         TabIndex        =   85
         Top             =   5040
         Width           =   615
      End
      Begin VB.Label Label27 
         Caption         =   "(0.1°)"
         Height          =   255
         Left            =   2160
         TabIndex        =   84
         Top             =   2520
         Width           =   855
      End
      Begin VB.Label Label_modbus_a_explain 
         Caption         =   "(r/min)/s"
         Height          =   255
         Left            =   2160
         TabIndex        =   83
         Top             =   2160
         Width           =   1215
      End
      Begin VB.Label Label25 
         Caption         =   "( r/min )"
         Height          =   255
         Left            =   2160
         TabIndex        =   82
         Top             =   1800
         Width           =   975
      End
      Begin VB.Label Label_modbus_EN_explain 
         Caption         =   "使能"
         Height          =   255
         Left            =   2160
         TabIndex        =   81
         Top             =   720
         Width           =   495
      End
      Begin VB.Label Label_modbus_enable_explain 
         Caption         =   "禁止"
         Height          =   255
         Left            =   2160
         TabIndex        =   80
         Top             =   360
         Width           =   735
      End
      Begin VB.Label Label_modbus_save_flag 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   79
         Top             =   5040
         Width           =   255
      End
      Begin VB.Label Label16 
         Caption         =   "参数保存:"
         Height          =   255
         Left            =   240
         TabIndex        =   78
         Top             =   5040
         Width           =   1215
      End
      Begin VB.Label Label_modbus_speed_start 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   77
         Top             =   2520
         Width           =   975
      End
      Begin VB.Label Label18 
         Caption         =   "弱磁角度:"
         Height          =   255
         Left            =   240
         TabIndex        =   76
         Top             =   2520
         Width           =   1335
      End
      Begin VB.Label Label_speed_Ki 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   75
         Top             =   3240
         Width           =   735
      End
      Begin VB.Label Label_20 
         Caption         =   "速度环Ki:"
         Height          =   255
         Left            =   240
         TabIndex        =   74
         Top             =   3240
         Width           =   1095
      End
      Begin VB.Label Label_modbus_a 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   54
         Top             =   2160
         Width           =   975
      End
      Begin VB.Label Label_modbus_speed 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   53
         Top             =   1800
         Width           =   975
      End
      Begin VB.Label Label_modbus_EN 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   52
         Top             =   720
         Width           =   975
      End
      Begin VB.Label Label_speed_Kp 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   51
         Top             =   2880
         Width           =   975
      End
      Begin VB.Label Label_modbus_enable 
         Caption         =   "0"
         ForeColor       =   &H00000080&
         Height          =   255
         Left            =   1560
         TabIndex        =   50
         Top             =   360
         Width           =   975
      End
      Begin VB.Label Label13 
         Caption         =   "电机加速度:"
         Height          =   255
         Left            =   240
         TabIndex        =   49
         Top             =   2160
         Width           =   1215
      End
      Begin VB.Label Label12 
         Caption         =   "目标转速:"
         Height          =   255
         Left            =   240
         TabIndex        =   48
         Top             =   1800
         Width           =   975
      End
      Begin VB.Label Label11 
         Caption         =   "EN(使能):"
         Height          =   255
         Left            =   240
         TabIndex        =   47
         Top             =   720
         Width           =   855
      End
      Begin VB.Label Label10 
         Caption         =   "速度环Kp:"
         Height          =   255
         Left            =   240
         TabIndex        =   46
         Top             =   2880
         Width           =   975
      End
      Begin VB.Label Label9 
         Caption         =   "PU(步数):"
         Height          =   255
         Index           =   0
         Left            =   240
         TabIndex        =   45
         Top             =   1080
         Width           =   975
      End
      Begin VB.Label Label8 
         Caption         =   "mosbus使能:"
         Height          =   255
         Left            =   240
         TabIndex        =   44
         Top             =   360
         Width           =   1095
      End
   End
   Begin VB.Timer Timer2 
      Interval        =   10
      Left            =   8760
      Top             =   5640
   End
   Begin VB.Timer Timer1 
      Interval        =   100
      Left            =   9240
      Top             =   5640
   End
   Begin MSCommLib.MSComm MSComm1 
      Left            =   9720
      Top             =   5520
      _ExtentX        =   1005
      _ExtentY        =   1005
      _Version        =   393216
      DTREnable       =   -1  'True
      InBufferSize    =   4096
      InputMode       =   1
   End
   Begin VB.PictureBox Pic1 
      Appearance      =   0  'Flat
      AutoRedraw      =   -1  'True
      BackColor       =   &H80000004&
      DrawMode        =   3  'Not Merge Pen
      FillStyle       =   0  'Solid
      ForeColor       =   &H80000008&
      Height          =   6015
      Left            =   600
      ScaleHeight     =   399
      ScaleMode       =   0  'User
      ScaleWidth      =   646
      TabIndex        =   1
      Top             =   240
      Width           =   9720
   End
   Begin VB.Frame Frame1 
      BackColor       =   &H8000000A&
      Caption         =   "波形显示"
      Height          =   9135
      Left            =   0
      TabIndex        =   0
      Top             =   0
      Width           =   10575
      Begin VB.Frame Frame7 
         Caption         =   "驱动器运行状态"
         Height          =   2175
         Left            =   8040
         TabIndex        =   86
         Top             =   6840
         Width           =   2415
         Begin VB.ComboBox baud_Combo 
            Height          =   300
            ItemData        =   "test1.frx":0126
            Left            =   840
            List            =   "test1.frx":0136
            TabIndex        =   114
            Text            =   "baud_combo"
            Top             =   1440
            Width           =   1455
         End
         Begin VB.CommandButton OPEN_COM_Command 
            Caption         =   "打开串口"
            Height          =   255
            Left            =   600
            TabIndex        =   105
            Top             =   1800
            Width           =   1215
         End
         Begin VB.ComboBox COM_Combo 
            Height          =   300
            ItemData        =   "test1.frx":0156
            Left            =   840
            List            =   "test1.frx":0191
            Style           =   2  'Dropdown List
            TabIndex        =   104
            Top             =   960
            Width           =   1455
         End
         Begin VB.Label Label31 
            Caption         =   "波特率："
            Height          =   375
            Left            =   120
            TabIndex        =   116
            Top             =   1440
            Width           =   735
         End
         Begin VB.Label Label3 
            Caption         =   "串口号："
            Height          =   375
            Left            =   120
            TabIndex        =   115
            Top             =   960
            Width           =   735
         End
         Begin VB.Label Label_waring_code_note 
            Alignment       =   2  'Center
            Caption         =   "label"
            Height          =   615
            Left            =   240
            TabIndex        =   90
            Top             =   360
            Width           =   2055
         End
      End
      Begin VB.Frame Frame_ch_set 
         Caption         =   "驱动器设置参数"
         Height          =   2175
         Left            =   3120
         TabIndex        =   28
         Top             =   6840
         Width           =   2295
         Begin VB.Label Label_drive_RunMode 
            Caption         =   "0"
            ForeColor       =   &H00000080&
            Height          =   255
            Left            =   960
            TabIndex        =   93
            Top             =   480
            Width           =   495
         End
         Begin VB.Label Label_drive_EN_explain 
            Caption         =   "使能"
            Height          =   255
            Left            =   1440
            TabIndex        =   36
            Top             =   1440
            Width           =   495
         End
         Begin VB.Label Label_drive_DIR_explain 
            Caption         =   "正转"
            Height          =   255
            Left            =   1440
            TabIndex        =   35
            Top             =   960
            Width           =   495
         End
         Begin VB.Label Label_drive_RunMode_explain 
            Caption         =   "位置模式"
            Height          =   255
            Left            =   1440
            TabIndex        =   34
            Top             =   480
            Width           =   735
         End
         Begin VB.Label Label_drive_EN 
            Caption         =   "0"
            ForeColor       =   &H00000080&
            Height          =   255
            Left            =   960
            TabIndex        =   33
            Top             =   1440
            Width           =   855
         End
         Begin VB.Label Label_drive_DIR 
            Caption         =   "0"
            ForeColor       =   &H00000080&
            Height          =   255
            Left            =   960
            TabIndex        =   32
            Top             =   960
            Width           =   855
         End
         Begin VB.Label Label_ch4_offset 
            Caption         =   "使能:"
            Height          =   255
            Left            =   240
            TabIndex        =   31
            Top             =   1440
            Width           =   975
         End
         Begin VB.Label Label_ch3_offset 
            Caption         =   "转向:"
            Height          =   255
            Left            =   240
            TabIndex        =   30
            Top             =   960
            Width           =   975
         End
         Begin VB.Label label_ch2_offset 
            Caption         =   "模式:"
            Height          =   255
            Left            =   240
            TabIndex        =   29
            Top             =   480
            Width           =   855
         End
      End
      Begin VB.Frame Frame2 
         Caption         =   "电机运行参数"
         Height          =   2175
         Left            =   240
         TabIndex        =   24
         Top             =   6840
         Width           =   2775
         Begin VB.Label Label23 
            Caption         =   "(r/min)"
            Height          =   255
            Left            =   1920
            TabIndex        =   89
            Top             =   1080
            Width           =   735
         End
         Begin VB.Label Label_motor_speed_current 
            Caption         =   "0"
            ForeColor       =   &H000000FF&
            Height          =   255
            Left            =   1320
            TabIndex        =   88
            Top             =   1080
            Width           =   495
         End
         Begin VB.Label Label21 
            Caption         =   "当前转速:"
            ForeColor       =   &H000000FF&
            Height          =   255
            Left            =   240
            TabIndex        =   87
            Top             =   1080
            Width           =   1095
         End
         Begin VB.Label Label19 
            Caption         =   "( V )"
            Height          =   255
            Left            =   1920
            TabIndex        =   73
            Top             =   1800
            Width           =   615
         End
         Begin VB.Label Label_motor_V 
            Caption         =   "0"
            ForeColor       =   &H00000080&
            Height          =   255
            Left            =   1320
            TabIndex        =   72
            Top             =   1800
            Width           =   615
         End
         Begin VB.Label Label17 
            Caption         =   "电压:"
            Height          =   255
            Left            =   240
            TabIndex        =   71
            Top             =   1800
            Width           =   735
         End
         Begin VB.Label Label7 
            Caption         =   "( % )"
            Height          =   255
            Left            =   1920
            TabIndex        =   42
            Top             =   720
            Width           =   615
         End
         Begin VB.Label Label6 
            Caption         =   "( ℃ )"
            Height          =   255
            Left            =   1920
            TabIndex        =   41
            Top             =   1440
            Width           =   615
         End
         Begin VB.Label Label5 
            Caption         =   "( A )"
            Height          =   255
            Left            =   1920
            TabIndex        =   40
            Top             =   360
            Width           =   615
         End
         Begin VB.Label Label_motor_pwm 
            Caption         =   "0"
            ForeColor       =   &H0000C000&
            Height          =   255
            Left            =   1320
            TabIndex        =   39
            Top             =   720
            Width           =   735
         End
         Begin VB.Label Label_motor_T 
            Caption         =   "0"
            ForeColor       =   &H00000080&
            Height          =   255
            Left            =   1320
            TabIndex        =   38
            Top             =   1440
            Width           =   735
         End
         Begin VB.Label Label_motor_I 
            Caption         =   "0"
            ForeColor       =   &H00FF0000&
            Height          =   255
            Index           =   0
            Left            =   1320
            TabIndex        =   37
            Top             =   360
            Width           =   735
         End
         Begin VB.Label Label_y_max 
            Caption         =   "输出脉宽:"
            ForeColor       =   &H0000C000&
            Height          =   255
            Left            =   240
            TabIndex        =   27
            Top             =   720
            Width           =   975
         End
         Begin VB.Label Label_x_min 
            Caption         =   "温度:"
            Height          =   255
            Left            =   240
            TabIndex        =   26
            Top             =   1440
            Width           =   615
         End
         Begin VB.Label Label_x_max 
            BackColor       =   &H80000004&
            Caption         =   "电流:"
            ForeColor       =   &H00FF0000&
            Height          =   255
            Left            =   240
            TabIndex        =   25
            Top             =   360
            Width           =   735
         End
      End
      Begin VB.Label Label_y 
         Caption         =   "8"
         Height          =   255
         Index           =   8
         Left            =   120
         TabIndex        =   23
         Top             =   240
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "7"
         Height          =   255
         Index           =   7
         Left            =   120
         TabIndex        =   22
         Top             =   960
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "6"
         Height          =   255
         Index           =   6
         Left            =   120
         TabIndex        =   21
         Top             =   1680
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "5"
         Height          =   255
         Index           =   5
         Left            =   120
         TabIndex        =   20
         Top             =   2400
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "4"
         Height          =   255
         Index           =   4
         Left            =   120
         TabIndex        =   19
         Top             =   3120
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "3"
         Height          =   255
         Index           =   3
         Left            =   120
         TabIndex        =   18
         Top             =   3960
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "2"
         Height          =   255
         Index           =   2
         Left            =   120
         TabIndex        =   17
         Top             =   4680
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "1"
         Height          =   255
         Index           =   1
         Left            =   120
         TabIndex        =   16
         Top             =   5400
         Width           =   615
      End
      Begin VB.Label Label_y 
         Caption         =   "0"
         Height          =   255
         Index           =   0
         Left            =   120
         TabIndex        =   15
         Top             =   6000
         Width           =   615
      End
      Begin VB.Label Label_x 
         Caption         =   "500"
         Height          =   255
         Index           =   10
         Left            =   10080
         TabIndex        =   14
         Top             =   6240
         Width           =   375
      End
      Begin VB.Label Label_x 
         Caption         =   "450"
         Height          =   375
         Index           =   9
         Left            =   9360
         TabIndex        =   13
         Top             =   6240
         Width           =   615
      End
      Begin VB.Label Label_x 
         Caption         =   "400"
         Height          =   255
         Index           =   8
         Left            =   8400
         TabIndex        =   12
         Top             =   6240
         Width           =   855
      End
      Begin VB.Label Label_x 
         Caption         =   "350"
         Height          =   255
         Index           =   7
         Left            =   7440
         TabIndex        =   11
         Top             =   6240
         Width           =   855
      End
      Begin VB.Label Label_x 
         Caption         =   "300"
         Height          =   375
         Index           =   6
         Left            =   6480
         TabIndex        =   10
         Top             =   6240
         Width           =   855
      End
      Begin VB.Label Label_x 
         Caption         =   "250"
         Height          =   375
         Index           =   5
         Left            =   5520
         TabIndex        =   9
         Top             =   6240
         Width           =   975
      End
      Begin VB.Label Label_x 
         Caption         =   "200"
         Height          =   375
         Index           =   4
         Left            =   4560
         TabIndex        =   8
         Top             =   6240
         Width           =   975
      End
      Begin VB.Label Label_x 
         Caption         =   "150"
         Height          =   495
         Index           =   3
         Left            =   3600
         TabIndex        =   7
         Top             =   6240
         Width           =   975
      End
      Begin VB.Label Label_x 
         Caption         =   "100"
         Height          =   375
         Index           =   2
         Left            =   2640
         TabIndex        =   6
         Top             =   6240
         Width           =   855
      End
      Begin VB.Label Label_x 
         Caption         =   "50"
         Height          =   255
         Index           =   1
         Left            =   1680
         TabIndex        =   5
         Top             =   6240
         Width           =   615
      End
      Begin VB.Label Label_x 
         Caption         =   "0"
         Height          =   255
         Index           =   0
         Left            =   720
         TabIndex        =   2
         Top             =   6240
         Width           =   735
      End
   End
   Begin VB.Label Label_modbus_data_display 
      Caption         =   "send:"
      Height          =   255
      Index           =   1
      Left            =   0
      TabIndex        =   119
      Top             =   0
      Width           =   9975
   End
   Begin VB.Label Label_x0 
      Caption         =   "0"
      Height          =   255
      Index           =   2
      Left            =   3120
      TabIndex        =   4
      Top             =   7440
      Width           =   735
   End
   Begin VB.Label Label_x0 
      Caption         =   "0"
      Height          =   255
      Index           =   1
      Left            =   4200
      TabIndex        =   3
      Top             =   7200
      Width           =   735
   End
End
Attribute VB_Name = "YZ_AIM_v2_61"
Attribute VB_GlobalNameSpace = False
Attribute VB_Creatable = False
Attribute VB_PredeclaredId = True
Attribute VB_Exposed = False
Private Type disp_modbus_data_type
   
    'read write
    modbus_enable As Long           '0. modbus控制使能，1使能，使能后PU,EN,DIR失效
    EN As Long                       '1.modbus控制 0:禁止    1:使能
    
    modbus_speed As Long             '2.单位 r/min
    modbus_a As Long                '3.加速度
    modbus_speed_start As Long      '4.起始速度 单位 r/min
    
    speed_kp  As Long                      '5.速度控制KP参数，范围0到10000代表0.0鍉10.0倍
    speed_ki As Long                        '6.速度控制Ki参数 0~2000 代表0到2S的积分时间
    position_kp As Long                     '7.位置控制Kp参数 60~3000  (r/min)/r
    
    speed_feedforward As Long           '8.速度前馈
    DIR_polarity As Long               '9.加速度前馈
    E_Gear_numerator As Long            '10.电子齿轮分子
    E_Gear_denominator As Long          '11.电子齿轮分母
    
    'read only
    PU As Double                          '12. 13 .剩余的脉冲数
    waring_code As Integer                 '14.报警代码
    system_I_dc As Integer                 '15.系统的直流电流
    speed_current As Integer                '16.当前速度 单位r/min
    system_v As Integer                     '17.系统电压
    system_T As Integer                     '18.系统温度
    
    system_v_pwm As Integer                 '19.系统输出的PWM
    modbus_save_flag As Long            '20.保存标志  0:不需保存 1: 需要保存  2:保存完毕
    device_add As Long
     PU_total As Double                          '22. 23 .剩余的脉冲数
    speed_filter As Long
    position_forward As Long
End Type

Private Type xy_set_type
    x_max   As Integer
    x_min   As Integer
    y_max   As Single
    y_min   As Single
End Type
Private Type modbus_type
    input_data As Byte      '当前输入的数据
    statue As Integer       '接收的状态
    device_add As Byte      '设备地址
    function_code As Byte   '功能码
    data_len As Long     '数据长度
    data() As Long       '保存数据
    data_index As Long   '数据索引
End Type
Private Type channel_set_type
    offset(3)   As Single
    gain(3)     As Single
    color(3)    As Single
End Type
Private Type plot_line_type
    data_before(3) As Integer
    data_now(3) As Integer
    Index  As Integer
End Type
Private Type CRC16_type
    data_in   As Byte
    result_hi As Byte
    result_lo As Byte
End Type
Dim crc16 As CRC16_type
Dim crc16_send As CRC16_type
Dim xy_set As xy_set_type
Dim modbus As modbus_type
Dim channel_set As channel_set_type
Dim plot_line As plot_line_type
Dim disp_modbus_data As disp_modbus_data_type

Dim uart_free_time As Integer

Dim modbus_RW As Boolean
'Dim modbus_save_flag As Boolean
Dim modbus_read_register_num As Byte
Dim modbus_send_finish_flag As Boolean
Dim modbus_search_add_times As Byte

Dim para_save_send_byte(7) As Byte

Dim para_save_send_mark As Byte
Dim para_save_send_mark_2 As Byte

Dim parameter_explain(18) As String
Dim turn_explain(1) As String
Dim EN_explain(1) As String
Dim RunMode_explain(2) As String
Dim save_flag_explain(2) As String
Dim waring_code_explain(10) As String
Dim COM_statue(2) As String
Dim DIR_polarity_explain(1) As String





Private Sub Command_modbus_send_Click()
    modbus_RW = True
End Sub

Private Sub PC_RW_modbus()

    Dim hexStr As String
    Dim i As Integer
    Dim strSentData As String
    Dim tmp As Long
    Dim send_byte(7) As Byte
    Dim send_byte_10(12) As Byte
'check modbus read time
    If Option_100ms.Value = True Then
        Timer1.Interval = 50
    End If
    If Option_200ms.Value = True Then
        Timer1.Interval = 100
    End If
    If Option_500ms.Value = True Then
        Timer1.Interval = 200
    End If
    If Option_1s.Value = True Then
        Timer1.Interval = 20
    End If
'check modbus read or write
    If modbus_RW = True Then        'write
        modbus_RW = False
        crc16_send.result_hi = &HFF
        crc16_send.result_lo = &HFF
        
        send_byte(0) = Val(Text_device_add.Text)    'device add
        send_byte_10(0) = Val(Text_device_add.Text)    'device add
        crc16_send.data_in = send_byte(0)
        Call CRC16_send_calc
        
        If Combo_send_type.ListIndex = 13 Then           'PU_all
            send_byte_10(1) = 16                            'function code
            crc16_send.data_in = send_byte_10(1)
            Call CRC16_send_calc
                      
            send_byte_10(2) = 0                            'function code
            crc16_send.data_in = send_byte_10(2)
            Call CRC16_send_calc
                    
            send_byte_10(3) = 12                            'function code
            crc16_send.data_in = send_byte_10(3)
            Call CRC16_send_calc
            
            send_byte_10(4) = 0                            'function code
            crc16_send.data_in = send_byte_10(4)
            Call CRC16_send_calc
            
            send_byte_10(5) = 2                            'function code
            crc16_send.data_in = send_byte_10(5)
            Call CRC16_send_calc
            
            send_byte_10(6) = 4                           'function code
            crc16_send.data_in = send_byte_10(6)
            Call CRC16_send_calc
            
            tmp = Val(Text_send_data.Text)
            
            
            
            If tmp < 0 Then
                tmp = &H7FFFFFFF + (tmp + 1)
                send_byte_10(9) = Fix(tmp / (256 * 65536)) + &H80 'register hi add
            Else
                send_byte_10(9) = Fix(tmp / (256 * 65536))  'register hi add
            End If
            
            send_byte_10(10) = Fix(tmp / 65536) Mod 256    'register lo add
           
            
            send_byte_10(7) = Fix(tmp / 256) Mod 256         'data hi
           
            
            send_byte_10(8) = Val(tmp) Mod 256         'data lo
           
            crc16_send.data_in = send_byte_10(7)
            Call CRC16_send_calc
            
            crc16_send.data_in = send_byte_10(8)
            Call CRC16_send_calc
            
            crc16_send.data_in = send_byte_10(9)
            Call CRC16_send_calc
            
             crc16_send.data_in = send_byte_10(10)
            Call CRC16_send_calc
            send_byte_10(11) = crc16_send.result_hi                   'CRC hi
            send_byte_10(12) = crc16_send.result_lo                    'CRC lo
            
         ElseIf Combo_send_type.ListIndex = 18 Then           'PU_all
            send_byte_10(1) = 16                            'function code
            crc16_send.data_in = send_byte_10(1)
            Call CRC16_send_calc
                      
            send_byte_10(2) = 0                            'function code
            crc16_send.data_in = send_byte_10(2)
            Call CRC16_send_calc
                    
            send_byte_10(3) = 22                            'function code
            crc16_send.data_in = send_byte_10(3)
            Call CRC16_send_calc
            
            send_byte_10(4) = 0                            'function code
            crc16_send.data_in = send_byte_10(4)
            Call CRC16_send_calc
            
            send_byte_10(5) = 2                            'function code
            crc16_send.data_in = send_byte_10(5)
            Call CRC16_send_calc
            
            send_byte_10(6) = 4                           'function code
            crc16_send.data_in = send_byte_10(6)
            Call CRC16_send_calc
            
            tmp = Val(Text_send_data.Text)
            
             If tmp < 0 Then
                tmp = &H7FFFFFFF + (tmp + 1)
                send_byte_10(9) = Fix(tmp / (256 * 65536)) + &H80 'register hi add
            Else
                send_byte_10(9) = Fix(tmp / (256 * 65536))  'register hi add
            End If
            
            send_byte_10(10) = Fix(tmp / 65536) Mod 256    'register lo add
           
            
            send_byte_10(7) = Fix(tmp / 256) Mod 256         'data hi
           
            
            send_byte_10(8) = Val(tmp) Mod 256         'data lo
           
            crc16_send.data_in = send_byte_10(7)
            Call CRC16_send_calc
            
            crc16_send.data_in = send_byte_10(8)
            Call CRC16_send_calc
            
            crc16_send.data_in = send_byte_10(9)
            Call CRC16_send_calc
            
             crc16_send.data_in = send_byte_10(10)
            Call CRC16_send_calc
            
            send_byte_10(11) = crc16_send.result_hi                   'CRC hi
            send_byte_10(12) = crc16_send.result_lo                    'CRC lo
        ElseIf Combo_send_type.ListIndex = 15 Then           'temperature compensation
            send_byte(1) = 121                             'function code
            crc16_send.data_in = send_byte(1)
            Call CRC16_send_calc
                        
            send_byte(2) = 0
            crc16_send.data_in = send_byte(2)
            Call CRC16_send_calc
            
            send_byte(3) = 0
            crc16_send.data_in = send_byte(3)
            Call CRC16_send_calc
            
            send_byte(4) = Fix((Val(Text_send_data.Text) / 256)) Mod 256         'data hi
            crc16_send.data_in = send_byte(4)
            Call CRC16_send_calc
            
            send_byte(5) = Val(Text_send_data.Text) Mod 256         'data lo
            crc16_send.data_in = send_byte(5)
            Call CRC16_send_calc
'        ElseIf Combo_send_type.ListIndex = 14 Then           'change device add
'            send_byte(1) = 122                             'function code
'            crc16_send.data_in = send_byte(1)
'            Call CRC16_send_calc
'
'            send_byte(2) = 0
'            crc16_send.data_in = send_byte(2)
'            Call CRC16_send_calc
'
'            send_byte(3) = 0
'            crc16_send.data_in = send_byte(3)
'            Call CRC16_send_calc
'
'            send_byte(4) = 0                                         'data hi
'            crc16_send.data_in = send_byte(4)
'            Call CRC16_send_calc
'
'            send_byte(5) = Val(Text_send_data.Text) Mod 256         'data lo
'            crc16_send.data_in = send_byte(5)
'            Call CRC16_send_calc
'            Text_device_add.Text = Str(send_byte(5))
        Else
            send_byte(1) = 6                             'function code
            crc16_send.data_in = send_byte(1)
            Call CRC16_send_calc
                        
            send_byte(2) = 0                             'register hi add
            crc16_send.data_in = send_byte(2)
            Call CRC16_send_calc
            
            If Combo_send_type.ListIndex = 12 Then
                send_byte(3) = 20
            ElseIf Combo_send_type.ListIndex = 16 Then
                send_byte(3) = 24
            ElseIf Combo_send_type.ListIndex = 17 Then
                send_byte(3) = 25
            ElseIf Combo_send_type.ListIndex = 14 Then
                send_byte(3) = 21
            Else
                send_byte(3) = Combo_send_type.ListIndex     'register lo add
            End If
            crc16_send.data_in = send_byte(3)
            Call CRC16_send_calc
            
            tmp = Val(Text_send_data.Text)
            If tmp < 0 Then
                tmp = &H7FFF + (tmp + 1)
                send_byte(4) = Fix(tmp / 256) + &H80  'register hi add
            Else
                send_byte(4) = Fix(tmp / 256)   'register hi add
            End If
            crc16_send.data_in = send_byte(4)
            Call CRC16_send_calc
            
            send_byte(5) = tmp Mod 256         'data lo
            crc16_send.data_in = send_byte(5)
            Call CRC16_send_calc
        End If
        
        
        If Combo_send_type.ListIndex = 13 Then
            MSComm1.Output = send_byte_10
            For i = 0 To 12
            hexStr = hexStr & Right$("0" & Hex(send_byte_10(i)), 2) & " "
                Next i
    
                    ' 记录并显示
            strSentData = strSentData & "[send:] " & hexStr & vbCrLf
            Text_send_desplay.Text = "已发送数据：" & vbCrLf & strSentData
            Text_send_desplay.SelStart = Len(Text_send_desplay.Text)  ' 自动滚动到底部
         ElseIf Combo_send_type.ListIndex = 18 Then
            MSComm1.Output = send_byte_10
            For i = 0 To 12
            hexStr = hexStr & Right$("0" & Hex(send_byte_10(i)), 2) & " "
                Next i
    
                    ' 记录并显示
            strSentData = strSentData & "[send:] " & hexStr & vbCrLf
            Text_send_desplay.Text = "已发送数据：" & vbCrLf & strSentData
            Text_send_desplay.SelStart = Len(Text_send_desplay.Text)  ' 自动滚动到底部
               
          Else
            send_byte(6) = crc16_send.result_hi                   'CRC hi
            send_byte(7) = crc16_send.result_lo                    'CRC lo
             MSComm1.Output = send_byte
            For i = 0 To 7
            hexStr = hexStr & Right$("0" & Hex(send_byte(i)), 2) & " "
                Next i
    
                    ' 记录并显示
            strSentData = strSentData & "[send:] " & hexStr & vbCrLf
            Text_send_desplay.Text = "已发送数据：" & vbCrLf & strSentData
            Text_send_desplay.SelStart = Len(Text_send_desplay.Text)  ' 自动滚动到底部
           
        End If
         modbus_send_finish_flag = True
    Else                'read
        If Check_begin_read_modbus.Value = 1 Then
            crc16_send.result_hi = &HFF
            crc16_send.result_lo = &HFF
            send_byte(0) = Val(Text_device_add.Text)    'device add
            crc16_send.data_in = send_byte(0)
            Call CRC16_send_calc
            
            send_byte(1) = 3                             'function code
            crc16_send.data_in = send_byte(1)
            Call CRC16_send_calc
            
            
            send_byte(2) = 0                             'register hi add
            crc16_send.data_in = send_byte(2)
            Call CRC16_send_calc
            
            send_byte(3) = 0                             'register lo add
            crc16_send.data_in = send_byte(3)
            Call CRC16_send_calc
            
            send_byte(4) = 0                             'register num hi
            crc16_send.data_in = send_byte(4)
            Call CRC16_send_calc
            
            send_byte(5) = modbus_read_register_num      'register num lo
            crc16_send.data_in = send_byte(5)
            Call CRC16_send_calc
            
            send_byte(6) = crc16_send.result_hi                   'data hi
            send_byte(7) = crc16_send.result_lo                    'data lo
            MSComm1.Output = send_byte
            
        End If
    End If

End Sub


Private Sub Form_Load()
    Dim i As Integer
    Combo_send_type.ListIndex = 0
    baud_Combo.ListIndex = 1
'parameter_explain init

    
    save_flag_explain(0) = "未保存"
    save_flag_explain(1) = "正在保存"
    save_flag_explain(2) = "已保存"

    parameter_explain(0) = "modbus使能 1:使能，0:禁止"
    parameter_explain(1) = "驱动器使能,0:禁止,1:使能 "
    parameter_explain(2) = "电机转速,单位r/min"
    parameter_explain(3) = "速度模式: 加速度 范围0~30000(r/min)/s"
    parameter_explain(4) = "弱磁角度,范围0~200  单位 0.1°"
    parameter_explain(5) = "速度控制KP参数,范围0鍉10000代表0.0到10.0倍"
    parameter_explain(6) = "速度环KI参数,范围5~2000 代表积分时间5ms~2000ms"
    parameter_explain(7) = "位置环KP参数,范围1~5000"
    parameter_explain(8) = "速度前馈,换算算法327=1(V/KRPM)"
    parameter_explain(9) = "脉冲模式DIR极性,0:负逻辑,1:正逻辑"
    parameter_explain(10) = "电子齿轮分子"
    parameter_explain(11) = "电子齿轮分母"
    parameter_explain(12) = "参数保存标志,0:不保存  1:保存"
    parameter_explain(13) = "PU 电机走的总步数"
    parameter_explain(14) = "修改设备地址,写入需要改成的地址"
    parameter_explain(15) = "修改温度"
    parameter_explain(16) = "静态最大输出，范围0~600，代表0~60.0%"
    parameter_explain(17) = "范围10~32768，对应0.1~360°，2为编码器跟随模式"
    parameter_explain(1) = "绝对前馈"
      
    RunMode_explain(0) = "速度模式"
    RunMode_explain(1) = "位置模式"
    RunMode_explain(2) = "参数测试"
    
    turn_explain(0) = "正转"
    turn_explain(1) = "反转"
    
    EN_explain(0) = "禁止"
    EN_explain(1) = "使能"
    
    COM_statue(0) = "打开串口"
    COM_statue(1) = "关闭串口"
    
    DIR_polarity_explain(0) = "负逻辑"
    DIR_polarity_explain(1) = "正逻辑"
    
    waring_code_explain(0) = "运行正常"
    waring_code_explain(1) = "驱动器过热( >70℃ )"
    waring_code_explain(2) = "写入flash失败"
    waring_code_explain(3) = "驱动器过热( >90℃),驱动器停机,温度恢复到70℃以下继续运行"
    waring_code_explain(4) = "驱动器过流报警,驱动器停机,重启后恢复正常"
    waring_code_explain(5) = "驱动器欠压报警,驱动器停机,电压正常后恢复"
    waring_code_explain(6) = "驱动器失速报警，电机负载过大，重启后恢复正常"
    waring_code_explain(7) = "驱动器脉冲输入频率过高,驱动器停机,重启后恢复正常"
    waring_code_explain(8) = "等待读取驱动器"
    waring_code_explain(9) = "无此串口端口号或串口被占用,请尝试打开其他串口端口"
    

   
    Call pic1_init
    Call pic1_xy_init
'    Call pic1_xy_init
'    For i = 0 To 3
'         Combo_ch_color(i).ListIndex = i
'    Next i
    plot_line.Index = 0
'read mudbus register num
    modbus_read_register_num = 26
    
    disp_modbus_data.waring_code = 8
    
    COM_Combo.ListIndex = 0
' SCI INIT
    On Error GoTo ErrorHandler
    MSComm1.CommPort = 1
    MSComm1.Settings = "19200,N,8,1 "
    MSComm1.InputMode = comInputModeBinary
    MSComm1.InputLen = 1
    MSComm1.RThreshold = 0
    MSComm1.SThreshold = 0
    If MSComm1.PortOpen = False Then
        MSComm1.PortOpen = True
        OPEN_COM_Command.Caption = COM_statue(1)
        COM_Combo.Enabled = False
        Check_begin_read_modbus.Value = 1
        Check_begin_read_modbus.Enabled = True
    End If
    MSComm1.InputMode = comInputModeBinary
Exit Sub
ErrorHandler:
    OPEN_COM_Command.Caption = COM_statue(0)
    COM_Combo.Enabled = True
    Check_begin_read_modbus.Value = 0
    Check_begin_read_modbus.Enabled = False
    disp_modbus_data.waring_code = 9
End Sub

Private Sub pic1_init()
'Frame_background 坐标系初始化
    Pic1.Cls
    Pic1.DrawWidth = 1
    Pic1.ScaleMode = 3
    Pic1.DrawStyle = 3
    Pic1.Line (0, Pic1.ScaleHeight / 2)-(Pic1.ScaleWidth, Pic1.ScaleHeight / 2), QBColor(8)
    Pic1.Line (Pic1.ScaleWidth / 2, 0)-(Pic1.ScaleWidth / 2, Pic1.ScaleHeight), QBColor(8)
    Pic1.DrawStyle = 2
    For i = 1 To 3
       Pic1.Line (0, Pic1.ScaleHeight * (i / 8))-(Pic1.ScaleWidth, Pic1.ScaleHeight * (i / 8)), QBColor(8)
    Next i
    For i = 5 To 7
       Pic1.Line (0, Pic1.ScaleHeight * (i / 8))-(Pic1.ScaleWidth, Pic1.ScaleHeight * (i / 8)), QBColor(8)
    Next i
    For i = 1 To 4
       Pic1.Line (Pic1.ScaleWidth * (i / 10), 0)-(Pic1.ScaleWidth * (i / 10), Pic1.ScaleHeight), QBColor(8)
    Next i
    For i = 1 To 4
       Pic1.Line (Pic1.ScaleWidth * (i / 10), 0)-(Pic1.ScaleWidth * (i / 10), Pic1.ScaleHeight), QBColor(8)
    Next i
    For i = 6 To 9
       Pic1.Line (Pic1.ScaleWidth * (i / 10), 0)-(Pic1.ScaleWidth * (i / 10), Pic1.ScaleHeight), QBColor(8)
    Next i
End Sub
Private Sub pic1_xy_init()
''xy坐标设置
    Dim i As Integer
'    xy_set.x_max = Val(Text_x_max.Text)
'    xy_set.x_min = Val(Text_x_min.Text)
'    xy_set.y_max = Val(Text_y_max.Text)
'    xy_set.y_min = -xy_set.y_max
    xy_set.x_max = 100
    xy_set.x_min = 0
    xy_set.y_max = 8
    xy_set.y_min = -xy_set.y_max
'    Text_y_min.Text = xy_set.y_min
'    For i = 0 To 3
'        channel_set.offset(i) = Text_ch_offset(i).Text
'        channel_set.gain(i) = Text_ch_gain(i).Text
'        channel_set.color(i) = 14 - Combo_ch_color(i).ListIndex
'    Next i
    For i = 0 To 10
        Label_x(i) = xy_set.x_min + (xy_set.x_max - xy_set.x_min) * (i / 10)
    Next i

    For i = 0 To 8
        Label_y(i) = xy_set.y_min + (xy_set.y_max - xy_set.y_min) * (i / 8)
    Next i
End Sub


Private Sub OPEN_COM_Command_Click()
    If OPEN_COM_Command.Caption = COM_statue(1) Then   '串口已打开
        If MSComm1.PortOpen = True Then
           MSComm1.PortOpen = False
           OPEN_COM_Command.Caption = COM_statue(0)
           COM_Combo.Enabled = True
           baud_Combo.Enabled = True
           Check_begin_read_modbus.Value = 0
           Check_begin_read_modbus.Enabled = False
           disp_modbus_data.waring_code = 9
        End If
    Else
' SCI INIT
    On Error GoTo ErrorHandler
    MSComm1.CommPort = COM_Combo.ListIndex + 1
    If baud_Combo.ListIndex = 0 Then
        MSComm1.Settings = "9600,N,8,1 "
    End If
    If baud_Combo.ListIndex = 1 Then
        MSComm1.Settings = "19200,N,8,1 "
    End If
    If baud_Combo.ListIndex = 2 Then
        MSComm1.Settings = "38400,N,8,1 "
    End If
    If baud_Combo.ListIndex = 3 Then
        MSComm1.Settings = "115200,N,8,1 "
    End If
    
    MSComm1.InputMode = comInputModeBinary
    MSComm1.InputLen = 1
    MSComm1.RThreshold = 0
    MSComm1.SThreshold = 0
    If MSComm1.PortOpen = False Then
        MSComm1.PortOpen = True
        OPEN_COM_Command.Caption = COM_statue(1)
        COM_Combo.Enabled = False
        baud_Combo.Enabled = False
        Check_begin_read_modbus.Value = 1
        Check_begin_read_modbus.Enabled = True
        disp_modbus_data.waring_code = 8
    End If
    MSComm1.InputMode = comInputModeBinary
Exit Sub
ErrorHandler:
    OPEN_COM_Command.Caption = COM_statue(0)
    COM_Combo.Enabled = True
    Check_begin_read_modbus.Value = 0
    Check_begin_read_modbus.Enabled = False
    disp_modbus_data.waring_code = 9
    End If
End Sub




Private Sub para_save_Click()

    
    crc16_send.result_hi = &HFF
    crc16_send.result_lo = &HFF
        
    para_save_send_byte(0) = Val(Text_device_add.Text)     'device add
    crc16_send.data_in = para_save_send_byte(0)
    Call CRC16_send_calc
    
    para_save_send_byte(1) = 6                             'function code
    crc16_send.data_in = para_save_send_byte(1)
    Call CRC16_send_calc
                        
    para_save_send_byte(2) = 0                             'register hi add
    crc16_send.data_in = para_save_send_byte(2)
    Call CRC16_send_calc
    
    para_save_send_byte(3) = 20                             'register hi add
    crc16_send.data_in = para_save_send_byte(3)
    Call CRC16_send_calc

    para_save_send_byte(4) = 0                             'register hi add
    crc16_send.data_in = para_save_send_byte(4)
    Call CRC16_send_calc
    
    para_save_send_byte(5) = 1                             'register hi add
    crc16_send.data_in = para_save_send_byte(5)
    Call CRC16_send_calc

    para_save_send_byte(6) = crc16_send.result_hi          'CRC hi
    para_save_send_byte(7) = crc16_send.result_lo          'CRC lo
    
   ' MSComm1.Output = para_save_send_byte
    para_save_send_mark = 10

End Sub

Private Sub Timer1_Timer()
    '处理modbus读写进程
     If para_save_send_mark = 10 Then
        MSComm1.Output = para_save_send_byte
        para_save_send_mark = para_save_send_mark - 1
     ElseIf para_save_send_mark > 0 Then
        para_save_send_mark = para_save_send_mark - 1
        If Label_modbus_save_flag.Caption = 1 Then
            MSComm1.Output = para_save_send_byte
            Label_modbus_save_flag.Caption = 0
        ElseIf Label_modbus_save_flag.Caption = 2 Then
            para_save_send_mark = 0
            Call PC_RW_modbus
        Else
            Call PC_RW_modbus
        End If
    Else
      Call PC_RW_modbus
    End If


    
    
    
    
    '处理参数显示
    Label_send_note.Caption = parameter_explain(Combo_send_type.ListIndex)
    Label_drive_RunMode_explain.Caption = RunMode_explain(Val(Label_drive_RunMode.Caption))

    'Label_drive_DIR_explain.Caption = turn_explain(Val(Label_drive_DIR.Caption))
    Label_drive_EN_explain.Caption = EN_explain(Val(Label_drive_EN.Caption))
    Label_modbus_enable_explain.Caption = EN_explain(Val(Label_modbus_enable.Caption) Mod 2)
    Label_modbus_EN_explain.Caption = EN_explain(Val(Label_modbus_EN.Caption))
    'Label_modbus_a_explain.Caption = a_explain(Val(Label_modbus_a.Caption))
    Label_modbus_save_flag_explain.Caption = save_flag_explain(Val(Label_modbus_save_flag.Caption))
    Label_waring_code_note.Caption = waring_code_explain(disp_modbus_data.waring_code)
    If disp_modbus_data.DIR_polarity > 1 Then
       disp_modbus_data.DIR_polarity = 1
    End If
    Label_DIR_polarity_explain.Caption = DIR_polarity_explain(disp_modbus_data.DIR_polarity)
    
  
   
 
    
    '处理低速demo进程
    'Call low_speed_demo_calc
    '处理定位demo进程
    modbus_search_add_times = modbus_search_add_times + 1
    If modbus_search_add_times > 2 Then
      modbus_search_add_times = 0
        If Check_search_read_modbus_add.Value = 1 Then
            If Text_device_add.Text < 255 Then
                Text_device_add.Text = Text_device_add.Text + 1
            End If
        End If
    End If
End Sub

'Private Sub Timer1_Timer()
'1S中断
 '   Call pic1_xy_init
'End Sub

Private Sub Timer2_Timer()
'2ms中断
    Dim n As Integer
    Dim get_data() As Byte
    
    n = MSComm1.InBufferCount
    If n < 1 Then
        uart_free_time = uart_free_time + 1
        If uart_free_time > 2 Then
           modbus.statue = 1
        End If
    End If
    
    While n > 0
        uart_free_time = 0
        get_data() = MSComm1.Input
        modbus.input_data = get_data(0)
        Call modbus_calc
        n = n - 1
    Wend
End Sub
Private Sub plot()
    Dim i As Integer
    Dim data As Double
    Dim x_index As Integer
    Dim x_index_next As Integer
    If plot_line.Index + (modbus.data_len / 4) > xy_set.x_max Then
        Call pic1_init  '坐标初始化
        plot_line.Index = 0
    End If
    Pic1.DrawStyle = 0
 '   For i = 0 To modbus.data_len / 4 - 1
        'data = modbus.data(i * 4) * channel_set.gain(0) + channel_set.offset(0)
        data = plot_line.data_now(0)
        x_index = plot_line.Index * Pic1.ScaleWidth / xy_set.x_max
        x_index_next = (plot_line.Index + 1) * Pic1.ScaleWidth / xy_set.x_max
        
        Pic1.Line (x_index, Pic1.ScaleHeight - (plot_line.data_before(0) / 32768 + 1) * Pic1.ScaleHeight / 2)-(x_index_next, Pic1.ScaleHeight - (data / 32768 + 1) * Pic1.ScaleHeight / 2), QBColor(6)
        plot_line.data_before(0) = plot_line.data_now(0)
        
        
        'data = modbus.data(i * 4 + 1) * channel_set.gain(1) + channel_set.offset(1)
        data = plot_line.data_now(1)
        Pic1.Line (x_index, Pic1.ScaleHeight - (plot_line.data_before(1) / 32768 + 1) * Pic1.ScaleHeight / 2)-(x_index_next, Pic1.ScaleHeight - (data / 32768 + 1) * Pic1.ScaleHeight / 2), QBColor(5)
        plot_line.data_before(1) = plot_line.data_now(1)
        
        'data = modbus.data(i * 4 + 2) * channel_set.gain(2) + channel_set.offset(2)
        data = plot_line.data_now(2)
        Pic1.Line (x_index, Pic1.ScaleHeight - (plot_line.data_before(2) / 32768 + 1) * Pic1.ScaleHeight / 2)-(x_index_next, Pic1.ScaleHeight - (data / 32768 + 1) * Pic1.ScaleHeight / 2), QBColor(3)
        plot_line.data_before(2) = plot_line.data_now(2)
        
        'data = modbus.data(i * 4 + 3) * channel_set.gain(3) + channel_set.offset(3)
        data = plot_line.data_now(3)
        Pic1.Line (x_index, Pic1.ScaleHeight - (plot_line.data_before(3) / 32768 + 1) * Pic1.ScaleHeight / 2)-(x_index_next, Pic1.ScaleHeight - (data / 32768 + 1) * Pic1.ScaleHeight / 2), QBColor(8)
        plot_line.data_before(3) = plot_line.data_now(3)
        plot_line.Index = plot_line.Index + 1
 '   Next i
End Sub
Private Sub modbus_calc()
    Select Case modbus.statue
        Case 1  '设备地址
            modbus.device_add = modbus.input_data
'            If modbus.device_add = Val(Text_device_add.Text) Then
                modbus.statue = 2
                crc16.result_hi = &HFF
                crc16.result_lo = &HFF
                crc16.data_in = modbus.input_data
               Call CRC16_calc
  '          End If
            
        Case 2  '功能码
            modbus.function_code = modbus.input_data
            If modbus.function_code = 3 Then
                modbus.statue = 3
                crc16.data_in = modbus.input_data
                Call CRC16_calc
            Else
                modbus.statue = 1
                crc16.result_hi = &HFF
                crc16.result_lo = &HFF
            End If
        Case 3  '数据长度
            modbus.data_len = modbus.input_data
            modbus.statue = 4
            modbus.data_index = 0
            crc16.data_in = modbus.input_data
            Call CRC16_calc
            ReDim modbus.data(modbus.data_len / 2) As Long
        Case 4 '数据位高位
            modbus.data(modbus.data_index) = modbus.input_data
            modbus.data(modbus.data_index) = modbus.data(modbus.data_index) * 256
            modbus.statue = 5
            crc16.data_in = modbus.input_data
            Call CRC16_calc
        Case 5 '数据位低位
            modbus.data(modbus.data_index) = modbus.data(modbus.data_index) + modbus.input_data
            modbus.statue = 4
            crc16.data_in = modbus.input_data
            Call CRC16_calc
            modbus.data_index = modbus.data_index + 1
            If modbus.data_index >= modbus.data_len / 2 Then
                modbus.statue = 6
            End If
        Case 6 'CRC高位
            If crc16.result_hi = modbus.input_data Then
                modbus.statue = 7
            End If
        Case 7 'CRC低位
            If crc16.result_lo = modbus.input_data Then
                modbus.statue = 1
                Select Case modbus.function_code
                       
                    Case 3
                        Call disp_modbus_calc
                        plot_line.data_now(0) = disp_modbus_data.system_I_dc
                        'plot_line.data_now(1) = disp_modbus_data.system_v_pwm
                        'plot_line.data_now(2) = disp_modbus_data.speed_current
         '               If disp_modbus_data.system_v_pwm > 3000 Then
         '                   disp_modbus_data.system_v_pwm = 3000
         '               End If
         '               If disp_modbus_data.system_v_pwm < -3000 Then
         '                   disp_modbus_data.system_v_pwm = -3000
         '               End If
         '               plot_line.data_now(1) = disp_modbus_data.system_v_pwm * 10
         '               If disp_modbus_data.speed_current > 3000 Then
         '                   disp_modbus_data.speed_current = 3000
          '              End If
         '               If disp_modbus_data.speed_current < -3000 Then
         '                   disp_modbus_data.speed_current = -3000
         '               End If
         '               plot_line.data_now(2) = disp_modbus_data.speed_current * 10
                        
                        plot_line.data_now(1) = disp_modbus_data.system_v_pwm
                        plot_line.data_now(2) = disp_modbus_data.speed_current
                        
                        plot_line.data_now(3) = disp_modbus_data.system_v
                        Call plot
                End Select
            End If
            
    End Select
End Sub
Private Sub disp_modbus_calc()
    disp_modbus_data.modbus_enable = modbus.data(0)
    disp_modbus_data.EN = modbus.data(1)
    
    If modbus.data(2) > 32767 Then
        disp_modbus_data.modbus_speed = (modbus.data(2) - 65536)
    Else
        disp_modbus_data.modbus_speed = modbus.data(2)
    End If
    
    disp_modbus_data.modbus_a = modbus.data(3)
    disp_modbus_data.modbus_speed_start = modbus.data(4)
    disp_modbus_data.speed_kp = modbus.data(5)
    disp_modbus_data.speed_ki = modbus.data(6)
    disp_modbus_data.position_kp = modbus.data(7)
    
    disp_modbus_data.speed_feedforward = modbus.data(8)
    disp_modbus_data.DIR_polarity = modbus.data(9)
    disp_modbus_data.E_Gear_numerator = modbus.data(10)
    disp_modbus_data.E_Gear_denominator = modbus.data(11)
    
    If modbus.data(13) > 32767 Then
       disp_modbus_data.PU = (modbus.data(13) - 32768) * 65536 + modbus.data(12)
       disp_modbus_data.PU = -((&H7FFFFFFF - disp_modbus_data.PU) + 1)
    Else
       disp_modbus_data.PU = modbus.data(13) * 65536 + modbus.data(12)
    End If
    
    disp_modbus_data.waring_code = 8
    Select Case modbus.data(14)
        Case 0
            disp_modbus_data.waring_code = 0
            Check_search_read_modbus_add.Value = 0
        Case &H10
            disp_modbus_data.waring_code = 1
        Case &H20
            disp_modbus_data.waring_code = 2
        Case &H11
            disp_modbus_data.waring_code = 3
        Case &H12
            disp_modbus_data.waring_code = 4
        Case &H13
            disp_modbus_data.waring_code = 5
        Case &H14
            disp_modbus_data.waring_code = 6
        Case &H15
            disp_modbus_data.waring_code = 7
    End Select
    If modbus.data(15) > 32767 Then
        disp_modbus_data.system_I_dc = (modbus.data(15) - 65536)
    Else
       disp_modbus_data.system_I_dc = modbus.data(15)
    End If
    
    If modbus.data(16) > 32767 Then
        disp_modbus_data.speed_current = (modbus.data(16) - 65536)
    Else
       disp_modbus_data.speed_current = modbus.data(16)
    End If
    
    If (modbus.data(17) > 32767) Then
        disp_modbus_data.system_v = (modbus.data(17) - 65536)
    Else
        disp_modbus_data.system_v = modbus.data(17)
    End If
    
    If modbus.data(18) > 32760 Then
        disp_modbus_data.system_T = 32760
    Else
       disp_modbus_data.system_T = modbus.data(18)
    End If
    
    If (modbus.data(19) > 32767) Then
        disp_modbus_data.system_v_pwm = (modbus.data(19) - 65536)
    Else
        disp_modbus_data.system_v_pwm = modbus.data(19)
    End If
    
    disp_modbus_data.modbus_save_flag = modbus.data(20)
    
    If modbus.data(23) > 32767 Then
       disp_modbus_data.PU_total = (modbus.data(23) - 32768) * 65536 + modbus.data(22)
       disp_modbus_data.PU_total = -((&H7FFFFFFF - disp_modbus_data.PU_total) + 1)
    Else
       disp_modbus_data.PU_total = modbus.data(23) * 65536 + modbus.data(22)
    End If
    
    disp_modbus_data.speed_filter = modbus.data(24)
    disp_modbus_data.position_forward = modbus.data(25)
    
    
    Label_motor_I(0).Caption = Round(CDbl(disp_modbus_data.system_I_dc) / 2000, 1)
    'Label_motor_I(0).Caption = disp_modbus_data.system_I_dc
    Label_motor_T.Caption = disp_modbus_data.system_T
    'Label_motor_V = Round(CDbl(disp_modbus_data.system_v / 256), 1)
    Label_motor_V.Caption = Round(CDbl(disp_modbus_data.system_v / 327), 1)
    'Label_motor_pwm = Round(CDbl(disp_modbus_data.system_v_pwm) * 100 / 32768, 1)
    Label_motor_pwm.Caption = Round(CDbl(disp_modbus_data.system_v_pwm) / 327, 1)
    Label_motor_speed_current.Caption = Round(CDbl(disp_modbus_data.speed_current / 10), 1)
    
    Label_drive_RunMode.Caption = Fix(disp_modbus_data.EN / 2) Mod 2
    Label_drive_EN.Caption = Fix(disp_modbus_data.EN / 4) Mod 2
    Label_drive_DIR.Caption = Fix(disp_modbus_data.EN / 8) Mod 2
    
    
    Label_modbus_enable.Caption = disp_modbus_data.modbus_enable
    Label_modbus_EN.Caption = disp_modbus_data.EN Mod 2
    Label_modbus_PU.Caption = disp_modbus_data.PU
    Label_modbus_speed.Caption = disp_modbus_data.modbus_speed
    Label_modbus_a.Caption = disp_modbus_data.modbus_a
    Label_modbus_speed_start.Caption = disp_modbus_data.modbus_speed_start
    Label_speed_Kp.Caption = disp_modbus_data.speed_kp
    Label_speed_Ki.Caption = disp_modbus_data.speed_ki
    Label_position_Kp.Caption = disp_modbus_data.position_kp
    Label_modbus_save_flag.Caption = disp_modbus_data.modbus_save_flag
    Label_speed_feedforward.Caption = Round(CDbl(disp_modbus_data.speed_feedforward) / 327, 1)
    Label_DIR_polarity.Caption = disp_modbus_data.DIR_polarity
    Label_E_Gear_numerator.Caption = disp_modbus_data.E_Gear_numerator
    LabelE_Gear_denominator.Caption = disp_modbus_data.E_Gear_denominator
    Label_modbus_PU_total.Caption = disp_modbus_data.PU_total
    Label_speed_filter.Caption = disp_modbus_data.speed_filter
    Label_position_forward.Caption = disp_modbus_data.position_forward
    
End Sub

Public Sub CRC16_calc()
    Dim iIndex As Long
    iIndex = crc16.result_hi Xor crc16.data_in
    crc16.result_hi = crc16.result_lo Xor GetCRCHi(iIndex)        '高位处理
    crc16.result_lo = GetCRCLo(iIndex)                                    '低位处理
    End Sub
Public Sub CRC16_send_calc()
    Dim iIndex As Long
    iIndex = crc16_send.result_hi Xor crc16_send.data_in
    crc16_send.result_hi = crc16_send.result_lo Xor GetCRCHi(iIndex)        '高位处理
    crc16_send.result_lo = GetCRCLo(iIndex)                                    '低位处理
    End Sub
    'CRC高位字节值表
Private Function GetCRCHi(Ind As Long) As Byte
      GetCRCHi = Choose(Ind + 1, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, _
                        &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40, &H1, &HC0, &H80, &H41, &H1, &HC0, &H80, &H41, &H0, &HC1, &H81, &H40)
    End Function
    'CRC低位字节值表
Private Function GetCRCLo(Ind As Long) As Byte
      GetCRCLo = Choose(Ind + 1, &H0, &HC0, &HC1, &H1, &HC3, &H3, &H2, &HC2, &HC6, &H6, &H7, &HC7, &H5, &HC5, &HC4, &H4, &HCC, &HC, &HD, &HCD, &HF, &HCF, &HCE, &HE, &HA, &HCA, &HCB, &HB, &HC9, &H9, &H8, &HC8, &HD8, &H18, &H19, &HD9, &H1B, &HDB, &HDA, &H1A, &H1E, &HDE, &HDF, &H1F, &HDD, &H1D, &H1C, &HDC, &H14, &HD4, &HD5, &H15, &HD7, &H17, &H16, &HD6, &HD2, &H12, &H13, &HD3, &H11, &HD1, &HD0, &H10, &HF0, &H30, &H31, &HF1, &H33, &HF3, &HF2, &H32, &H36, &HF6, &HF7, &H37, &HF5, &H35, &H34, &HF4, &H3C, &HFC, &HFD, &H3D, &HFF, &H3F, &H3E, &HFE, &HFA, &H3A, &H3B, &HFB, &H39, &HF9, &HF8, &H38, &H28, &HE8, &HE9, &H29, &HEB, &H2B, &H2A, &HEA, &HEE, &H2E, &H2F, &HEF, &H2D, &HED, &HEC, &H2C, &HE4, &H24, &H25, &HE5, &H27, &HE7, &HE6, &H26, &H22, &HE2, &HE3, &H23, &HE1, &H21, &H20, &HE0, &HA0, &H60, _
                        &H61, &HA1, &H63, &HA3, &HA2, &H62, &H66, &HA6, &HA7, &H67, &HA5, &H65, &H64, &HA4, &H6C, &HAC, &HAD, &H6D, &HAF, &H6F, &H6E, &HAE, &HAA, &H6A, &H6B, &HAB, &H69, &HA9, &HA8, &H68, &H78, &HB8, &HB9, &H79, &HBB, &H7B, &H7A, &HBA, &HBE, &H7E, &H7F, &HBF, &H7D, &HBD, &HBC, &H7C, &HB4, &H74, &H75, &HB5, &H77, &HB7, &HB6, &H76, &H72, &HB2, &HB3, &H73, &HB1, &H71, &H70, &HB0, &H50, &H90, &H91, &H51, &H93, &H53, &H52, &H92, &H96, &H56, &H57, &H97, &H55, &H95, &H94, &H54, &H9C, &H5C, &H5D, &H9D, &H5F, &H9F, &H9E, &H5E, &H5A, &H9A, &H9B, &H5B, &H99, &H59, &H58, &H98, &H88, &H48, &H49, &H89, &H4B, &H8B, &H8A, &H4A, &H4E, &H8E, &H8F, &H4F, &H8D, &H4D, &H4C, &H8C, &H44, &H84, &H85, &H45, &H87, &H47, &H46, &H86, &H82, &H42, &H43, &H83, &H41, &H81, &H80, &H40)
    End Function


